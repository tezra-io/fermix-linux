//! Fetching the feed and a package over HTTP(S), against servers on 127.0.0.1
//! started here: the feed read stops past its cap, a download lands only when
//! its size and sha256 match (rule 7) and is deleted otherwise, every wait is
//! bounded, and TLS trusts only the roots it is given. Nothing leaves the host.

use fermix_client::update::feed::{Package, FEED_CAP};
use fermix_client::update::fetch::{native_roots, FetchConfig, FetchError, Fetcher, Timeouts};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

const TLS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/update-tls");
/// How many connections one test server answers before it stops.
const CONNECTIONS: usize = 8;
/// The longest request head a test server reads.
const HEAD_CAP: usize = 16 * 1024;

#[derive(Clone)]
struct Route {
    status: u16,
    headers: Vec<(&'static str, String)>,
    body: Vec<u8>,
}

fn ok(body: &[u8]) -> Route {
    Route {
        status: 200,
        headers: Vec::new(),
        body: body.to_vec(),
    }
}

fn redirect(to: &str) -> Route {
    Route {
        status: 302,
        headers: vec![("Location", to.to_owned())],
        body: Vec::new(),
    }
}

/// Serves `routes` on a port of its own, plain or over TLS, for up to
/// `CONNECTIONS` connections. Returns the base URL.
fn serve(routes: HashMap<&'static str, Route>, tls: Option<Arc<rustls::ServerConfig>>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let scheme = if tls.is_some() { "https" } else { "http" };
    thread::spawn(move || {
        for stream in listener.incoming().take(CONNECTIONS) {
            let Ok(stream) = stream else { return };
            let (routes, tls) = (routes.clone(), tls.clone());
            thread::spawn(move || answer(stream, &routes, tls));
        }
    });
    format!("{scheme}://127.0.0.1:{port}")
}

fn answer(
    stream: TcpStream,
    routes: &HashMap<&'static str, Route>,
    tls: Option<Arc<rustls::ServerConfig>>,
) {
    match tls {
        Some(config) => {
            let connection = rustls::ServerConnection::new(config).unwrap();
            respond(rustls::StreamOwned::new(connection, stream), routes);
        }
        None => respond(stream, routes),
    }
}

fn respond(mut stream: impl Read + Write, routes: &HashMap<&'static str, Route>) {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") && head.len() < HEAD_CAP {
        if stream.read(&mut byte).unwrap_or(0) == 0 {
            return;
        }
        head.push(byte[0]);
    }
    let head = String::from_utf8_lossy(&head);
    let path = head.split_whitespace().nth(1).unwrap_or("/");
    let route = routes.get(path).cloned().unwrap_or(Route {
        status: 404,
        headers: Vec::new(),
        body: b"no such file".to_vec(),
    });
    let mut reply = format!(
        "HTTP/1.1 {} X\r\nContent-Length: {}\r\nConnection: close\r\n",
        route.status,
        route.body.len()
    );
    for (name, value) in &route.headers {
        reply.push_str(&format!("{name}: {value}\r\n"));
    }
    reply.push_str("\r\n");
    // The client may hang up first (a capped read, a cancel): that is its answer.
    let _ = stream.write_all(reply.as_bytes());
    let _ = stream.write_all(&route.body);
    let _ = stream.flush();
}

fn tls_server() -> Arc<rustls::ServerConfig> {
    let cert = CertificateDer::from(std::fs::read(format!("{TLS}/test-server.der")).unwrap());
    let key =
        PrivatePkcs8KeyDer::from(std::fs::read(format!("{TLS}/test-server-key.pk8")).unwrap());
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(vec![cert], PrivateKeyDer::Pkcs8(key))
        .unwrap();
    Arc::new(config)
}

fn test_ca() -> Vec<u8> {
    std::fs::read(format!("{TLS}/test-ca.der")).unwrap()
}

fn quick() -> Timeouts {
    Timeouts {
        connect: Duration::from_secs(5),
        feed: Duration::from_secs(5),
        per_read: Duration::from_millis(500),
        download: Duration::from_secs(10),
    }
}

/// Plain HTTP, for the tests about bytes rather than transport.
fn plain() -> Fetcher {
    Fetcher::new(FetchConfig {
        roots: vec![test_ca()],
        https_only: false,
        proxy_from_env: false,
        timeouts: quick(),
    })
    .unwrap()
}

fn package(body: &[u8], size: u64) -> Package {
    Package {
        file: "fermix-desktop_0.4.0_amd64.deb".into(),
        url: "unused".into(),
        size,
        sha256: format!("{:x}", Sha256::digest(body)),
    }
}

fn leftovers(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect()
}

#[test]
fn the_feed_is_read_from_its_server() {
    let base = serve(
        HashMap::from([("/appcast.json", ok(b"{\"format\": 1}"))]),
        None,
    );
    let bytes = plain().feed(&format!("{base}/appcast.json")).unwrap();
    assert_eq!(bytes, b"{\"format\": 1}");
}

#[test]
fn reading_the_feed_stops_one_byte_past_its_cap() {
    let huge = vec![b' '; FEED_CAP + 4096];
    let base = serve(HashMap::from([("/appcast.json", ok(&huge))]), None);
    let bytes = plain().feed(&format!("{base}/appcast.json")).unwrap();
    assert_eq!(bytes.len(), FEED_CAP + 1);
}

#[test]
fn a_server_refusal_is_its_status() {
    let base = serve(HashMap::new(), None);
    assert_eq!(
        plain().feed(&format!("{base}/appcast.json")),
        Err(FetchError::Status(404))
    );
}

#[test]
fn redirects_are_followed_as_github_s_downloads_need() {
    let body = b"the package bytes";
    let routes = HashMap::from([
        (
            "/releases/download/v0.4.0/p.deb",
            redirect("/objects/p.deb"),
        ),
        ("/objects/p.deb", ok(body)),
    ]);
    let base = serve(routes, None);
    let dir = tempfile::tempdir().unwrap();
    let to = dir.path().join("p.deb");
    let url = format!("{base}/releases/download/v0.4.0/p.deb");
    plain()
        .download(
            &url,
            &package(body, body.len() as u64),
            &to,
            &mut |_| {},
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(std::fs::read(&to).unwrap(), body);
}

#[test]
fn rule_7_a_download_matching_its_size_and_sha256_lands_whole() {
    let body: Vec<u8> = (0..300_000u32).map(|n| (n % 251) as u8).collect();
    let base = serve(HashMap::from([("/p.deb", ok(&body))]), None);
    let dir = tempfile::tempdir().unwrap();
    let to = dir.path().join("p.deb");
    let mut seen = Vec::new();
    plain()
        .download(
            &format!("{base}/p.deb"),
            &package(&body, body.len() as u64),
            &to,
            &mut |got| seen.push(got),
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(std::fs::read(&to).unwrap(), body);
    assert_eq!(leftovers(dir.path()), ["p.deb"], "no partial file is left");
    assert!(seen.windows(2).all(|w| w[0] < w[1]), "progress only grows");
    assert_eq!(seen.last(), Some(&(body.len() as u64)));
}

#[test]
fn rule_7_a_download_with_another_digest_is_deleted_and_refused() {
    let base = serve(HashMap::from([("/p.deb", ok(b"tampered"))]), None);
    let dir = tempfile::tempdir().unwrap();
    let to = dir.path().join("p.deb");
    let answer = plain().download(
        &format!("{base}/p.deb"),
        &package(b"original", 8),
        &to,
        &mut |_| {},
        &AtomicBool::new(false),
    );
    assert_eq!(answer, Err(FetchError::Digest));
    assert!(leftovers(dir.path()).is_empty(), "nothing is kept");
}

#[test]
fn rule_7_a_download_longer_or_shorter_than_its_size_is_deleted_and_refused() {
    let body = b"twelve bytes";
    let base = serve(HashMap::from([("/p.deb", ok(body))]), None);
    let dir = tempfile::tempdir().unwrap();
    let to = dir.path().join("p.deb");
    let long = plain().download(
        &format!("{base}/p.deb"),
        &package(&body[..10], 10),
        &to,
        &mut |_| {},
        &AtomicBool::new(false),
    );
    assert_eq!(long, Err(FetchError::TooLong { size: 10 }));
    assert!(leftovers(dir.path()).is_empty());
    let short = plain().download(
        &format!("{base}/p.deb"),
        &package(b"twelve bytes and more", 21),
        &to,
        &mut |_| {},
        &AtomicBool::new(false),
    );
    assert_eq!(short, Err(FetchError::Short { got: 12, size: 21 }));
    assert!(leftovers(dir.path()).is_empty());
}

#[test]
fn a_cancelled_download_stops_and_leaves_nothing() {
    let body = vec![1u8; 2_000_000];
    let base = serve(HashMap::from([("/p.deb", ok(&body))]), None);
    let dir = tempfile::tempdir().unwrap();
    let to = dir.path().join("p.deb");
    let cancel = AtomicBool::new(false);
    let answer = plain().download(
        &format!("{base}/p.deb"),
        &package(&body, body.len() as u64),
        &to,
        &mut |_| cancel.store(true, Ordering::SeqCst),
        &cancel,
    );
    assert_eq!(answer, Err(FetchError::Cancelled));
    assert!(leftovers(dir.path()).is_empty());
}

#[test]
fn a_server_that_never_answers_times_out() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    // Accepted and held open, never answered.
    let held = thread::spawn(move || listener.accept().map(|(stream, _)| stream));
    let started = Instant::now();
    let answer = plain().feed(&format!("http://127.0.0.1:{port}/appcast.json"));
    assert!(
        matches!(answer, Err(FetchError::Transport(_))),
        "{answer:?}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(8),
        "bounded by the timeouts"
    );
    drop(held);
}

#[test]
fn https_is_trusted_through_the_roots_given() {
    let base = serve(
        HashMap::from([("/appcast.json", ok(b"signed"))]),
        Some(tls_server()),
    );
    let trusted = Fetcher::new(FetchConfig {
        roots: vec![test_ca()],
        https_only: true,
        proxy_from_env: false,
        timeouts: quick(),
    })
    .unwrap();
    assert_eq!(
        trusted.feed(&format!("{base}/appcast.json")).unwrap(),
        b"signed"
    );
    let leaf_only = std::fs::read(format!("{TLS}/test-server.der")).unwrap();
    let untrusted = Fetcher::new(FetchConfig {
        roots: vec![leaf_only],
        https_only: true,
        proxy_from_env: false,
        timeouts: quick(),
    })
    .unwrap();
    let refused = untrusted.feed(&format!("{base}/appcast.json"));
    assert!(
        matches!(refused, Err(FetchError::Transport(_))),
        "{refused:?}"
    );
}

#[test]
fn https_only_refuses_plain_http() {
    let base = serve(HashMap::from([("/appcast.json", ok(b"x"))]), None);
    let strict = Fetcher::new(FetchConfig {
        roots: vec![test_ca()],
        https_only: true,
        proxy_from_env: false,
        timeouts: quick(),
    })
    .unwrap();
    let answer = strict.feed(&format!("{base}/appcast.json"));
    assert!(
        matches!(answer, Err(FetchError::Transport(_))),
        "{answer:?}"
    );
}

#[test]
fn no_roots_is_refused_before_any_request() {
    let config = FetchConfig {
        roots: Vec::new(),
        https_only: true,
        proxy_from_env: false,
        timeouts: quick(),
    };
    assert!(matches!(Fetcher::new(config), Err(FetchError::NoRoots(_))));
}

#[test]
fn the_host_store_has_roots() {
    let (roots, _unreadable) = native_roots().expect("the host has a CA store");
    assert!(!roots.is_empty());
}

#[test]
fn production_timeouts_are_bounded() {
    let production = Timeouts::PRODUCTION;
    assert!(production.connect <= Duration::from_secs(30));
    assert!(production.feed <= Duration::from_secs(60));
    assert!(production.per_read <= Duration::from_secs(60));
    assert!(production.download <= Duration::from_secs(2 * 60 * 60));
    let config = FetchConfig::production(vec![test_ca()]);
    assert!(config.https_only);
    assert!(config.proxy_from_env);
}

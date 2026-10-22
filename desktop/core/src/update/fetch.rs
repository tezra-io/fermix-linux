//! The feed and the packages over HTTPS: ureq on rustls with the ring
//! provider, trusting the roots it is handed (the host's own store, in the
//! app), so the window links no libssl. Blocking, with every wait bounded;
//! the app runs each call off its main loop.

use crate::update::feed::{Package, FEED_CAP};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use ureq::tls::{Certificate, RootCerts, TlsConfig};

/// One read of a download's body at most.
const CHUNK: usize = 64 * 1024;
/// Redirects are followed this far: GitHub's release downloads take one.
const MAX_REDIRECTS: u32 = 5;

/// How long each part of a request may take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timeouts {
    /// Name resolution, the TCP connection and the TLS handshake.
    pub connect: Duration,
    /// The whole feed request, body included.
    pub feed: Duration,
    /// Any one read: a download that stalls this long fails.
    pub per_read: Duration,
    /// The whole download, however fast it moves.
    pub download: Duration,
}

impl Timeouts {
    pub const PRODUCTION: Timeouts = Timeouts {
        connect: Duration::from_secs(15),
        feed: Duration::from_secs(30),
        per_read: Duration::from_secs(30),
        download: Duration::from_secs(60 * 60),
    };
}

pub struct FetchConfig {
    /// The roots TLS trusts, each a DER certificate.
    pub roots: Vec<Vec<u8>>,
    /// Refuse plain http, redirects included.
    pub https_only: bool,
    /// Go through the proxy `HTTPS_PROXY`/`ALL_PROXY` names, as other apps do.
    pub proxy_from_env: bool,
    pub timeouts: Timeouts,
}

impl FetchConfig {
    pub fn production(roots: Vec<Vec<u8>>) -> FetchConfig {
        FetchConfig {
            roots,
            https_only: true,
            proxy_from_env: true,
            timeouts: Timeouts::PRODUCTION,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchError {
    /// The host's CA store gave no certificate, or none was handed in.
    NoRoots(String),
    /// The request did not complete: no network, DNS, TLS, a timeout.
    Transport(String),
    /// The server answered with this HTTP status.
    Status(u16),
    /// The download went past the size the feed names.
    TooLong {
        size: u64,
    },
    /// The download ended before the size the feed names.
    Short {
        got: u64,
        size: u64,
    },
    /// The download's sha256 is not the feed's.
    Digest,
    Cancelled,
    /// The file could not be written, renamed or removed.
    Disk(String),
}

/// The host's CA certificates, as DER, and what could not be read of the
/// store (a broken link, an unreadable file), for the log.
pub fn native_roots() -> Result<(Vec<Vec<u8>>, Vec<String>), FetchError> {
    let found = rustls_native_certs::load_native_certs();
    let unreadable: Vec<String> = found.errors.iter().map(|e| e.to_string()).collect();
    if found.certs.is_empty() {
        return Err(FetchError::NoRoots(unreadable.join("; ")));
    }
    let roots = found.certs.iter().map(|c| c.as_ref().to_vec()).collect();
    Ok((roots, unreadable))
}

pub struct Fetcher {
    agent: ureq::Agent,
    timeouts: Timeouts,
}

impl Fetcher {
    pub fn new(config: FetchConfig) -> Result<Fetcher, FetchError> {
        if config.roots.is_empty() {
            return Err(FetchError::NoRoots("no root certificate was given".into()));
        }
        let roots: Vec<Certificate<'static>> = config
            .roots
            .iter()
            .map(|der| Certificate::from_der(der).to_owned())
            .collect();
        let tls = TlsConfig::builder()
            .root_certs(RootCerts::Specific(Arc::new(roots)))
            .unversioned_rustls_crypto_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .build();
        let proxy = if config.proxy_from_env {
            ureq::Proxy::try_from_env()
        } else {
            None
        };
        let agent = ureq::Agent::config_builder()
            .tls_config(tls)
            .https_only(config.https_only)
            .proxy(proxy)
            .max_redirects(MAX_REDIRECTS)
            .timeout_connect(Some(config.timeouts.connect))
            .timeout_per_read(Some(config.timeouts.per_read))
            .build()
            .new_agent();
        Ok(Fetcher {
            agent,
            timeouts: config.timeouts,
        })
    }

    /// The served feed, read to one byte past `FEED_CAP` at most, so
    /// `read_feed` refuses one that is longer without the rest being read.
    pub fn feed(&self, url: &str) -> Result<Vec<u8>, FetchError> {
        let response = self
            .agent
            .get(url)
            .config()
            .timeout_global(Some(self.timeouts.feed))
            .build()
            .call()
            .map_err(fetch_error)?;
        let cap = u64::try_from(FEED_CAP).expect("the cap fits") + 1;
        let mut bytes = Vec::new();
        response
            .into_body()
            .into_reader()
            .take(cap)
            .read_to_end(&mut bytes)
            .map_err(|e| FetchError::Transport(e.to_string()))?;
        Ok(bytes)
    }

    /// Downloads `package` from `url` to `to` (rule 7): it lands only once its
    /// size and sha256 are the feed's, and on any failure nothing is left.
    /// `progress` hears the bytes so far after each read; `cancel` is read
    /// before each one.
    pub fn download(
        &self,
        url: &str,
        package: &Package,
        to: &Path,
        progress: &mut dyn FnMut(u64),
        cancel: &AtomicBool,
    ) -> Result<(), FetchError> {
        let partial = partial_path(to);
        let fetched = self.download_to(url, package, &partial, progress, cancel);
        let Err(refused) = fetched.and_then(|()| fs::rename(&partial, to).map_err(disk)) else {
            return Ok(());
        };
        match remove_if_present(&partial).and_then(|()| remove_if_present(to)) {
            Ok(()) => Err(refused),
            Err(FetchError::Disk(e)) => Err(FetchError::Disk(format!(
                "{e}, so a refused download ({refused:?}) is still on disk"
            ))),
            Err(other) => Err(other),
        }
    }

    fn download_to(
        &self,
        url: &str,
        package: &Package,
        partial: &Path,
        progress: &mut dyn FnMut(u64),
        cancel: &AtomicBool,
    ) -> Result<(), FetchError> {
        if let Some(dir) = partial.parent() {
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(dir)
                .map_err(disk)?;
        }
        let response = self
            .agent
            .get(url)
            .config()
            .timeout_global(Some(self.timeouts.download))
            .build()
            .call()
            .map_err(fetch_error)?;
        let mut body = response.into_body().into_reader();
        // A new file, never one already there or through a link planted in its place.
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(partial)
            .map_err(|e| FetchError::Disk(format!("{}: {e}", partial.display())))?;
        let digest = copy_counted(&mut body, &mut file, package.size, progress, cancel)?;
        file.sync_all().map_err(disk)?;
        if digest != package.sha256 {
            return Err(FetchError::Digest);
        }
        Ok(())
    }
}

/// Copies at most `size` bytes, hashing them, and refuses a body that is
/// longer or shorter. Every read returns a byte or ends the body, so the
/// loop's cap is never what stops it.
fn copy_counted(
    body: &mut impl Read,
    file: &mut File,
    size: u64,
    progress: &mut dyn FnMut(u64),
    cancel: &AtomicBool,
) -> Result<String, FetchError> {
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; CHUNK];
    let mut got: u64 = 0;
    for _ in 0..=size.saturating_add(1) {
        if cancel.load(Ordering::SeqCst) {
            return Err(FetchError::Cancelled);
        }
        let read = body
            .read(&mut buffer)
            .map_err(|e| FetchError::Transport(e.to_string()))?;
        if read == 0 {
            return finished(got, size, hasher);
        }
        got += u64::try_from(read).expect("a read fits in u64");
        if got > size {
            return Err(FetchError::TooLong { size });
        }
        hasher.update(&buffer[..read]);
        file.write_all(&buffer[..read]).map_err(disk)?;
        progress(got);
    }
    Err(FetchError::TooLong { size })
}

fn finished(got: u64, size: u64, hasher: Sha256) -> Result<String, FetchError> {
    if got < size {
        return Err(FetchError::Short { got, size });
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// Where a download is written until it is checked.
fn partial_path(to: &Path) -> PathBuf {
    let mut name = to.as_os_str().to_owned();
    name.push(".partial");
    PathBuf::from(name)
}

fn remove_if_present(path: &Path) -> Result<(), FetchError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(disk(e)),
    }
}

fn fetch_error(error: ureq::Error) -> FetchError {
    match error {
        ureq::Error::StatusCode(status) => FetchError::Status(status),
        other => FetchError::Transport(other.to_string()),
    }
}

fn disk(error: std::io::Error) -> FetchError {
    FetchError::Disk(error.to_string())
}

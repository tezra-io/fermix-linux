//! The packaging gates.
//!
//! Everything the `fermix-desktop` package installs besides the binary is a
//! plain text file that nothing at build time reads back, and every way one of
//! them can be wrong is silent: a launcher that does not appear, a window that
//! does not group under its icon, an application that is invisible in the
//! software centres, a second launch that opens a second window, a version
//! relation that cannot be satisfied.
//!
//! `desktop-file-validate` and `appstreamcli` check two of these files against
//! their own specifications. What they cannot check is whether the files agree
//! with this crate and with each other, which is what these gates are for.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// The one application identity, and the one place this file writes it down.
const APP_ID: &str = "io.tezra.Fermix";

/// The icon sizes the package installs. The same list lives in
/// `scripts/build_packages.sh` and in `scripts/render_icons.sh`, and the gate
/// below holds the checked-in set equal to it.
const ICON_SIZES: &[u32] = &[16, 22, 24, 32, 48, 64, 128, 256];

/// Every file the package installs that this crate can name exactly, in two
/// halves.
///
/// Every file this repository authors and the package installs, as the path it
/// is installed at.
///
/// The window's half only. The package carries the engine too, but not one line
/// of the engine's file list is written here or in the nFPM template: it is
/// authored by the verified engine archive's own `nfpm-contents.yaml` and
/// spliced into the rendered configuration by `scripts/build_packages.sh`. That
/// is what makes "the engine layout is identical in both packages" true by
/// construction, and it is why an engine release that adds a packaged file
/// needs no edit in this repository at all.
///
/// Not quite everything the window installs, either: the private toolkit
/// runtime is a tree of some hundreds of shared objects the build copies
/// wholesale, so it is asserted below by the shape of its paths rather than
/// file by file.
const DESKTOP_PATHS: &[&str] = &[
    "/usr/bin/fermix-desktop",
    "/usr/lib/fermix-desktop/bin/fermix-desktop",
    "/usr/share/doc/fermix-desktop/runtime-manifest.json",
    "/usr/share/applications/io.tezra.Fermix.desktop",
    "/usr/share/metainfo/io.tezra.Fermix.metainfo.xml",
    "/usr/share/dbus-1/services/io.tezra.Fermix.service",
    "/usr/lib/systemd/user/app-io.tezra.Fermix.service",
    "/usr/share/icons/hicolor/16x16/apps/io.tezra.Fermix.png",
    "/usr/share/icons/hicolor/22x22/apps/io.tezra.Fermix.png",
    "/usr/share/icons/hicolor/24x24/apps/io.tezra.Fermix.png",
    "/usr/share/icons/hicolor/32x32/apps/io.tezra.Fermix.png",
    "/usr/share/icons/hicolor/48x48/apps/io.tezra.Fermix.png",
    "/usr/share/icons/hicolor/64x64/apps/io.tezra.Fermix.png",
    "/usr/share/icons/hicolor/128x128/apps/io.tezra.Fermix.png",
    "/usr/share/icons/hicolor/256x256/apps/io.tezra.Fermix.png",
    "/usr/share/icons/hicolor/scalable/apps/io.tezra.Fermix.svg",
    "/usr/share/icons/hicolor/symbolic/apps/io.tezra.Fermix-symbolic.svg",
    "/usr/share/fermix-desktop/build.json",
    "/usr/share/doc/fermix-desktop/copyright",
];

fn repository() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the crate sits two directories inside the repository")
        .to_path_buf()
}

fn packaging(relative: &str) -> String {
    let path = repository().join("packaging").join(relative);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("packaging/{relative} reads: {error}"))
}

/// One `Key=value` out of a desktop entry, a D-Bus service file or a unit.
fn entry_value(body: &str, key: &str) -> Option<String> {
    body.lines()
        .map(str::trim)
        .filter(|line| !line.starts_with('#'))
        .find_map(|line| line.strip_prefix(&format!("{key}=")))
        .map(str::to_string)
}

/// A file's directives, with every comment line taken out. A comment explaining
/// which unit the engine owns is not this unit owning it.
fn directives(body: &str) -> String {
    body.lines()
        .map(str::trim)
        .filter(|line| !line.starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The sentences a person reads, with the comments and the markup taken out. A
/// comment is written for whoever edits the file next, and holding it to the
/// catalogue's rules would only teach people to write worse comments.
fn prose(body: &str) -> String {
    let mut kept = String::with_capacity(body.len());
    let mut rest = body;

    while let Some(open) = rest.find("<!--") {
        kept.push_str(&rest[..open]);
        match rest[open..].find("-->") {
            Some(close) => rest = &rest[open + close + 3..],
            None => return directives(&kept),
        }
    }
    kept.push_str(rest);
    directives(&kept)
}

/// The text inside one XML element, with its whitespace collapsed.
fn element(body: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = body.find(&open)? + open.len();
    let end = body[start..].find(&close)? + start;
    Some(
        body[start..end]
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" "),
    )
}

/// A PNG's own idea of how big it is.
fn png_size(path: &Path) -> (u32, u32) {
    let bytes = std::fs::read(path).unwrap_or_else(|error| panic!("{path:?} reads: {error}"));
    assert!(bytes.len() >= 24, "{path:?} is too short to be a PNG");
    assert_eq!(
        &bytes[..8],
        b"\x89PNG\r\n\x1a\n",
        "{path:?} is not a PNG at all"
    );
    let width = u32::from_be_bytes(bytes[16..20].try_into().expect("four bytes"));
    let height = u32::from_be_bytes(bytes[20..24].try_into().expect("four bytes"));
    (width, height)
}

// ---------------------------------------------------------------------------
// One version
// ---------------------------------------------------------------------------

#[test]
fn the_metainfo_names_the_crates_own_version_and_no_other() {
    let metainfo = packaging(&format!("{APP_ID}.metainfo.xml"));

    let versions: BTreeSet<String> = metainfo
        .lines()
        .filter_map(|line| {
            line.trim()
                .strip_prefix("<release version=\"")
                .map(str::to_string)
        })
        .map(|rest| rest.split('"').next().unwrap_or_default().to_string())
        .collect();

    assert_eq!(
        versions.len(),
        1,
        "the metainfo carries {} release entries, and one version ships at a time: {versions:?}",
        versions.len()
    );
    assert_eq!(
        versions.iter().next().map(String::as_str),
        Some(env!("CARGO_PKG_VERSION")),
        "the metainfo's release and the crate's version are one version, and the tag is the same one"
    );
}

#[test]
fn no_sentence_a_person_reads_carries_a_version_number() {
    // The release element is the one place a version is written; every sentence
    // around it has to stay true when the next one is cut.
    let metainfo = packaging(&format!("{APP_ID}.metainfo.xml"));

    for tag in ["name", "summary"] {
        let text = element(&metainfo, tag).unwrap_or_else(|| panic!("the metainfo has a <{tag}>"));
        assert!(
            !text.chars().any(|character| character.is_ascii_digit()),
            "the metainfo's {tag} carries a number: {text}"
        );
    }

    let description_start = metainfo.find("<description>").expect("a description");
    let description_end = metainfo.find("</description>").expect("a description");
    let description = &metainfo[description_start..description_end];
    assert!(
        !description
            .chars()
            .any(|character| character.is_ascii_digit()),
        "the metainfo's description carries a number"
    );
}

// ---------------------------------------------------------------------------
// The launcher
// ---------------------------------------------------------------------------

#[test]
fn the_desktop_entry_carries_every_key_the_design_names() {
    let entry = packaging(&format!("{APP_ID}.desktop"));

    for (key, value) in [
        ("Type", "Application"),
        ("Name", "Fermix"),
        ("Exec", "/usr/bin/fermix-desktop %u"),
        ("Icon", APP_ID),
        ("Terminal", "false"),
        ("Categories", "Utility;GTK;"),
        ("Keywords", "assistant;agent;fermix;"),
        ("StartupWMClass", APP_ID),
        ("DBusActivatable", "true"),
        ("SingleMainWindow", "true"),
        ("MimeType", "x-scheme-handler/fermix;"),
    ] {
        assert_eq!(
            entry_value(&entry, key).as_deref(),
            Some(value),
            "the desktop entry's {key}"
        );
    }

    assert!(
        entry.starts_with("[Desktop Entry]\n") || entry.contains("\n[Desktop Entry]\n"),
        "the desktop entry has no [Desktop Entry] group"
    );
    assert!(
        !entry.contains("[Desktop Action"),
        "the desktop entry declares an action, and v1 has none"
    );
}

#[test]
fn the_launchers_tooltip_is_the_software_centres_subtitle() {
    let entry = packaging(&format!("{APP_ID}.desktop"));
    let metainfo = packaging(&format!("{APP_ID}.metainfo.xml"));

    assert_eq!(
        entry_value(&entry, "Comment"),
        element(&metainfo, "summary"),
        "the desktop entry's Comment and the metainfo's summary are one sentence"
    );
}

// ---------------------------------------------------------------------------
// Activation
// ---------------------------------------------------------------------------

#[test]
fn the_bus_activation_runs_through_the_unit_beside_it() {
    let service = packaging(&format!("dbus/{APP_ID}.service"));
    let unit = packaging(&format!("systemd/app-{APP_ID}.service"));

    assert_eq!(entry_value(&service, "Name").as_deref(), Some(APP_ID));
    assert_eq!(
        entry_value(&service, "SystemdService").as_deref(),
        Some(format!("app-{APP_ID}.service").as_str()),
        "the D-Bus service must name the unit, or a cold-session launch never runs through the user manager and the portal identity is not derivable"
    );

    assert_eq!(entry_value(&unit, "Type").as_deref(), Some("dbus"));
    assert_eq!(entry_value(&unit, "BusName").as_deref(), Some(APP_ID));
    assert_eq!(entry_value(&unit, "Slice").as_deref(), Some("app.slice"));

    // Both doors start the same process the same way, and both pass the flag
    // that makes it wait for the bus rather than open a window and exit.
    for (what, body, key) in [
        ("the D-Bus service", &service, "Exec"),
        ("the unit", &unit, "ExecStart"),
    ] {
        assert_eq!(
            entry_value(body, key).as_deref(),
            Some("/usr/bin/fermix-desktop --gapplication-service"),
            "{what}'s {key}"
        );
    }
}

#[test]
fn the_activation_unit_starts_the_interface_and_nothing_else() {
    let unit = packaging(&format!("systemd/app-{APP_ID}.service"));

    // The engine's unit is fermix.service, the fermix package owns it, and the
    // packaged command line reconciles it. This unit touching it in any way is
    // the arrangement the whole design exists to delete.
    let unit = directives(&unit);
    for forbidden in [
        "fermix.service",
        "ExecStartPre",
        "ExecStopPost",
        "Requires=",
    ] {
        assert!(
            !unit.contains(forbidden),
            "the activation unit carries {forbidden}, and it starts the interface and nothing else"
        );
    }
    assert!(
        !unit.contains("[Install]"),
        "the activation unit is bus-activated, so nothing enables it"
    );
}

// ---------------------------------------------------------------------------
// The icon set
// ---------------------------------------------------------------------------

#[test]
fn the_icon_set_is_every_size_the_package_installs() {
    let icons = repository().join("packaging/icons/hicolor");

    for size in ICON_SIZES {
        let path = icons.join(format!("{size}x{size}/apps/{APP_ID}.png"));
        assert!(
            path.exists(),
            "no {size}x{size} raster; run scripts/render_icons.sh"
        );
        assert_eq!(
            png_size(&path),
            (*size, *size),
            "the {size}x{size} raster is not {size} by {size} pixels"
        );
    }

    for (kind, relative) in [
        ("scalable", format!("scalable/apps/{APP_ID}.svg")),
        ("symbolic", format!("symbolic/apps/{APP_ID}-symbolic.svg")),
    ] {
        let path = icons.join(&relative);
        assert!(
            path.exists(),
            "no {kind} icon at packaging/icons/hicolor/{relative}"
        );
        let body = std::fs::read_to_string(&path).expect("the icon reads");
        assert!(body.contains("<svg"), "the {kind} icon is not an SVG");
    }
}

#[test]
fn the_packaged_icons_are_the_applications_own() {
    // One source, rendered. A packaged icon that has drifted from the one the
    // window draws is a dash icon that is not the window's icon.
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/icons");
    let packaged = repository().join("packaging/icons/hicolor");

    for (from, to) in [
        (
            format!("{APP_ID}.svg"),
            format!("scalable/apps/{APP_ID}.svg"),
        ),
        (
            format!("{APP_ID}-symbolic.svg"),
            format!("symbolic/apps/{APP_ID}-symbolic.svg"),
        ),
    ] {
        assert_eq!(
            std::fs::read(source.join(&from)).expect("the source icon reads"),
            std::fs::read(packaged.join(&to)).expect("the packaged icon reads"),
            "packaging/icons/hicolor/{to} is not resources/icons/{from}; run scripts/render_icons.sh"
        );
    }
}

// ---------------------------------------------------------------------------
// The nFPM configuration
// ---------------------------------------------------------------------------

/// Every `dst:` the configuration names.
fn template_destinations(template: &str) -> BTreeSet<String> {
    template
        .lines()
        .filter_map(|line| line.trim().strip_prefix("dst: ").map(str::to_string))
        .map(|value| value.trim_matches('"').to_string())
        .collect()
}

#[test]
fn the_template_installs_every_file_the_package_claims() {
    let template = packaging("nfpm-fermix-desktop.yaml.tmpl");
    let destinations = template_destinations(&template);

    for claimed in DESKTOP_PATHS {
        assert!(
            destinations.contains(*claimed),
            "the nFPM configuration installs nothing at {claimed}"
        );
    }
}

/// The files and directories the engine authors, which this repository must not
/// author a second time.
///
/// A path ending in `/` is a whole directory and everything under it; anything
/// else is one exact file. The distinction matters more than it looks: every
/// engine-owned root here has a desktop-owned near-namesake one character
/// longer, `/usr/lib/fermix` against `/usr/lib/fermix-desktop` and
/// `/usr/share/doc/fermix` against `/usr/share/doc/fermix-desktop`, and a
/// prefix test written without the separator would forbid the window's own
/// files.
const ENGINE_OWNED: &[&str] = &[
    "/usr/bin/fermix",
    "/usr/lib/fermix/",
    "/usr/lib/systemd/user/fermix.service",
    "/usr/share/fermix/",
    "/usr/share/doc/fermix/",
    "/usr/share/bash-completion/completions/fermix",
    "/usr/share/zsh/site-functions/_fermix",
    "/usr/share/fish/vendor_completions.d/fermix.fish",
    "/usr/share/man/man1/fermix.1.gz",
];

/// Whether one installed path belongs to the engine rather than to the window.
fn engine_owned(path: &str) -> bool {
    ENGINE_OWNED
        .iter()
        .any(|owned| match owned.strip_suffix('/') {
            Some(root) => path.starts_with(&format!("{root}/")),
            None => path == *owned,
        })
}

#[test]
fn the_template_leaves_every_engine_owned_path_to_the_engine() {
    // Two authors, one package, and this is the line between them. The engine's
    // file list comes from the verified archive's own `nfpm-contents.yaml` and
    // is spliced into the rendered configuration by the build; the template
    // states the window's files and nothing else. A `dst:` here under an
    // engine-owned root is the two authors colliding, and nFPM would either
    // carry the file twice or take whichever entry it read last.
    let template = packaging("nfpm-fermix-desktop.yaml.tmpl");

    let trespassing: Vec<String> = template_destinations(&template)
        .into_iter()
        .filter(|path| engine_owned(path))
        .collect();

    assert!(
        trespassing.is_empty(),
        "the nFPM template declares engine-owned paths, which the engine archive \
         already authors: {trespassing:?}"
    );
}

/// The comment line the build replaces with the engine's own contents.
///
/// Agreed with slice 4 and duplicated on purpose: `scripts/splice_engine_contents.py`
/// holds the same string in its `MARKER` constant and refuses the build if the
/// rendered configuration carries it any number of times other than once. Two
/// ends asserting the same literal is what makes a silent drift in either one a
/// red build rather than a package with no engine in it.
const ENGINE_SPLICE_MARKER: &str = "# >>> engine contents, spliced from the verified archive";

#[test]
fn the_template_marks_the_one_place_the_engine_contents_are_spliced_in() {
    let template = packaging("nfpm-fermix-desktop.yaml.tmpl");

    // Twice would splice the engine's whole file list in twice; not at all
    // would build a `fermix-desktop` carrying no engine, and neither is a
    // shape the rest of these gates would notice.
    assert_eq!(
        template.matches(ENGINE_SPLICE_MARKER).count(),
        1,
        "the template carries the engine splice marker {} times, and the build \
         refuses anything but once",
        template.matches(ENGINE_SPLICE_MARKER).count()
    );

    // On a line of its own, because the splice replaces the whole line. A
    // marker with anything after it would take that text out of the file along
    // with the marker.
    assert!(
        template
            .lines()
            .any(|line| line.trim() == ENGINE_SPLICE_MARKER),
        "the engine splice marker is not a line of its own"
    );
}

#[test]
fn the_line_between_the_two_authors_falls_where_the_names_nearly_collide() {
    // The rule above is a prefix test over paths that differ by one word, so it
    // is worth proving in both directions rather than trusting the reading.
    for engine in [
        "/usr/bin/fermix",
        "/usr/lib/fermix/cosign",
        "/usr/lib/fermix/runtime-payload/libc-musl-abc123.so",
        "/usr/lib/systemd/user/fermix.service",
        "/usr/share/fermix/engine.json",
        "/usr/share/doc/fermix/copyright",
        "/usr/share/man/man1/fermix.1.gz",
    ] {
        assert!(engine_owned(engine), "{engine} is the engine's");
    }

    for desktop in [
        "/usr/bin/fermix-desktop",
        "/usr/lib/fermix-desktop/bin/fermix-desktop",
        "/usr/lib/fermix-desktop/lib/libgtk-4.so.1",
        "/usr/lib/systemd/user/app-io.tezra.Fermix.service",
        "/usr/share/fermix-desktop/build.json",
        "/usr/share/doc/fermix-desktop/copyright",
        "/usr/share/doc/fermix-desktop/runtime-manifest.json",
    ] {
        assert!(!engine_owned(desktop), "{desktop} is the window's");
    }

    // Every path the window claims passes the same test, so the two lists
    // cannot drift into each other without this failing.
    for claimed in DESKTOP_PATHS {
        assert!(
            !engine_owned(claimed),
            "{claimed} is claimed by the window and forbidden to it"
        );
    }
}

#[test]
fn the_template_installs_the_private_toolkit_runtime() {
    // It cannot be named file by file: it is a tree of some hundreds of shared
    // objects the build copies out of the runtime image, so what is asserted is
    // that the tree is shipped at all.
    //
    // The engine's musl loader payload is deliberately not asserted here any
    // more. It lives under `/usr/lib/fermix/runtime-payload/`, which is an
    // engine-owned root, so it arrives with the rest of the engine's contents
    // and a `dst:` for it in this template would be the collision the gate
    // above forbids.
    let template = packaging("nfpm-fermix-desktop.yaml.tmpl");

    assert!(
        template_destinations(&template)
            .iter()
            .any(|path| path.starts_with("/usr/lib/fermix-desktop/lib")),
        "the nFPM configuration ships no private toolkit runtime, and the window links one"
    );
}

#[test]
fn the_installed_binary_is_reached_through_a_link_into_the_private_prefix() {
    // `/usr/bin/fermix-desktop` is a symbolic link, never a wrapper script: the
    // real ELF resolves its libraries through `$ORIGIN/../lib`, and a wrapper
    // would mean the process the desktop sees is not the one the launcher,
    // `StartupWMClass` and the activation unit all name.
    let template = packaging("nfpm-fermix-desktop.yaml.tmpl");

    let link = template
        .split("dst: ")
        .find(|block| block.starts_with("/usr/bin/fermix-desktop"))
        .expect("the configuration installs /usr/bin/fermix-desktop");

    assert!(
        link.contains("type: symlink"),
        "/usr/bin/fermix-desktop is not a symbolic link, and the design has no launcher script"
    );
    assert!(
        link.contains("lib/fermix-desktop/bin/fermix-desktop"),
        "/usr/bin/fermix-desktop points somewhere other than the private prefix"
    );
}

#[test]
fn the_template_declares_the_glibc_floor_and_the_engine_alternative() {
    // The directives, with the comment lines taken out. Several assertions
    // below are that a relation is *absent*, and a comment explaining why the
    // package no longer requires a host libgtk is not the package requiring
    // one. Reading the raw file would turn every such explanation into a
    // failure and teach whoever hits it to delete the explanation.
    let template = directives(&packaging("nfpm-fermix-desktop.yaml.tmpl"));

    // One package per family now, and it is the alternative to the engine-only
    // `fermix` rather than a layer on top of it. `Provides` keeps anything that
    // depends on the engine by name satisfied; `Conflicts` makes the two
    // mutually exclusive; `Replaces` is what lets dpkg hand `/usr/bin/fermix`
    // from one package to the other in a single transaction. There is
    // deliberately no `Obsoletes`, which would convert a headless install into
    // a desktop one on the next upgrade.
    for declaration in ["fermix (= {{VERSION}})", "fermix = {{VERSION}}"] {
        assert!(
            template.contains(declaration),
            "the nFPM configuration does not provide {declaration}"
        );
    }
    for relation in ["conflicts:", "replaces:"] {
        assert!(
            template.contains(relation),
            "the nFPM configuration declares no {relation}, so the two packages are not exclusive"
        );
    }
    assert!(
        !template.contains("obsoletes"),
        "the nFPM configuration obsoletes the engine-only package, which would pull a \
         private toolkit onto a headless host on the next upgrade"
    );

    // The floor is glibc, and it is a declared relation so that apt and dnf
    // refuse before anything unpacks rather than failing at exec.
    for floor in ["libc6 (>= 2.34)", "libc.so.6(GLIBC_2.34)"] {
        assert!(
            template.contains(floor),
            "the nFPM configuration does not declare the glibc floor {floor}"
        );
    }

    // The toolkit is carried, not required. A host relation on it would refuse
    // to install on the oldest supported target, which is the one target the
    // private runtime exists for.
    for host_toolkit in [
        "libgtk-4-1",
        "libadwaita-1-0",
        "gtk4 >=",
        "libadwaita >=",
        "librsvg2-common",
        "webp-pixbuf-loader",
    ] {
        assert!(
            !template.contains(host_toolkit),
            "the nFPM configuration still requires {host_toolkit} from the host, and the \
             package carries its own"
        );
    }

    // Section 2.1: the bundled GSettings backend is a client of the host's
    // dconf service and of the host's schemas, and on a session with no
    // Settings portal it is the only path the window has to the theme.
    for host_relation in ["dconf", "gsettings-desktop-schemas"] {
        assert!(
            template.contains(host_relation),
            "the nFPM configuration does not declare {host_relation}"
        );
    }

    assert!(
        template.contains("release: \"\""),
        "the nFPM configuration sets a release, and neither package may carry a Debian revision"
    );
    assert!(
        template.contains("version_schema: none"),
        "the nFPM configuration lets nFPM reshape the version"
    );
}

#[test]
fn every_placeholder_in_the_template_is_one_the_build_fills() {
    let template = packaging("nfpm-fermix-desktop.yaml.tmpl");
    // No `ENGINE_CONTENTS` here, deliberately. The engine's contents are
    // spliced in after the render, against the verified staging directory, at
    // the comment marker asserted above; the render step has no value it could
    // substitute for a placeholder. A placeholder in this set that nothing
    // fills is exactly what this gate exists to catch.
    let filled: BTreeSet<&str> = ["ARCH", "VERSION", "STAGE", "POSTINSTALL", "POSTREMOVE"]
        .into_iter()
        .collect();

    let mut found = BTreeSet::new();
    let mut rest = template.as_str();
    while let Some(open) = rest.find("{{") {
        let after = &rest[open + 2..];
        let close = after.find("}}").expect("a placeholder closes");
        found.insert(after[..close].to_string());
        rest = &after[close + 2..];
    }

    for placeholder in &found {
        assert!(
            filled.contains(placeholder.as_str()),
            "the template carries {{{{{placeholder}}}}} and the build fills {filled:?}"
        );
    }
    assert!(
        !found.is_empty(),
        "the template carries no placeholder at all"
    );
}

// ---------------------------------------------------------------------------
// One identity, and one voice
// ---------------------------------------------------------------------------

#[test]
fn every_packaging_file_carries_the_one_identity_and_no_other() {
    for relative in [
        format!("{APP_ID}.desktop"),
        format!("{APP_ID}.metainfo.xml"),
        format!("dbus/{APP_ID}.service"),
        format!("systemd/app-{APP_ID}.service"),
    ] {
        let body = packaging(&relative);
        assert!(
            body.contains(APP_ID),
            "packaging/{relative} does not carry {APP_ID}"
        );
        // Built rather than written out: `scripts/check_app_identity.sh` greps
        // the whole tree for a second identity, and a test that spelled one out
        // would be the thing it found.
        for stray in [
            format!("{APP_ID}Pet"),
            "org.tezra".into(),
            "io.fermix".into(),
        ] {
            assert!(
                !body.contains(&stray),
                "packaging/{relative} carries a second identity, {stray}"
            );
        }
    }
}

#[test]
fn the_packaged_sentences_follow_the_same_copy_rules_as_the_catalogue() {
    for relative in [
        format!("{APP_ID}.desktop"),
        format!("{APP_ID}.metainfo.xml"),
        "nfpm-fermix-desktop.yaml.tmpl".to_string(),
    ] {
        let body = prose(&packaging(&relative));
        for (character, what) in [('\u{2014}', "an em dash"), ('!', "an exclamation mark")] {
            assert!(
                !body.contains(character),
                "packaging/{relative} carries {what}"
            );
        }
        assert!(
            !body.to_lowercase().contains("please wait"),
            "packaging/{relative} carries a waiting sentence"
        );
    }
}

// ---------------------------------------------------------------------------
// The workflows
// ---------------------------------------------------------------------------
//
// A workflow is another declarative file nothing at build time reads back, and
// every way one can be wrong is silent until a release needs it. These gates
// hold the set of them to what this repository declares, hold every action to
// an immutable pin, and hold the two workflows that build packages to one
// shared definition of how a package is built.

/// Every workflow, by file name, with the sentence that says what it is for.
const WORKFLOWS: &[(&str, &str)] = &[
    ("app.yml", "the build and the gates, on every push"),
    ("contract.yml", "the vendored contract pin, on its own"),
    (
        "packages.yml",
        "both packages, on demand and on a packaging change",
    ),
    (
        "release-fermix-desktop.yml",
        "one tag, two architectures, one release page",
    ),
    (
        "runtime.yml",
        "the private toolkit runtime image, built and published when its lock file changes",
    ),
    (
        "engine-release.yml",
        "an engine release, taken as a pin bump, and the tag that follows it onto main",
    ),
    (
        "runtime-watch.yml",
        "the bundled runtime's upstream releases and advisories, weekly",
    ),
];

/// The composite actions this repository defines, which are what stop a
/// workflow from being a copy of another one.
const ACTIONS: &[&str] = &["build-packages"];

fn workflow(name: &str) -> String {
    let path = repository().join(".github/workflows").join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!(".github/workflows/{name} reads: {error}"))
}

/// Every file under `.github/` that carries workflow syntax.
fn declarative_github_files() -> Vec<(String, String)> {
    let mut files: Vec<(String, String)> = WORKFLOWS
        .iter()
        .map(|(name, _)| (format!(".github/workflows/{name}"), workflow(name)))
        .collect();

    for action in ACTIONS {
        let relative = format!(".github/actions/{action}/action.yml");
        let body = std::fs::read_to_string(repository().join(&relative))
            .unwrap_or_else(|error| panic!("{relative} reads: {error}"));
        files.push((relative, body));
    }
    files
}

#[test]
fn the_workflows_on_disk_are_exactly_the_ones_declared_here() {
    let directory = repository().join(".github/workflows");
    let mut present: BTreeSet<String> = std::fs::read_dir(&directory)
        .expect("the workflow directory reads")
        .map(|entry| {
            entry
                .expect("a workflow file")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    present.retain(|name| name.ends_with(".yml") || name.ends_with(".yaml"));

    let declared: BTreeSet<String> = WORKFLOWS
        .iter()
        .map(|(name, _)| (*name).to_string())
        .collect();

    assert_eq!(
        present, declared,
        "a workflow was added or removed without this gate being told, so nothing \
         holds it to the rules below"
    );
}

#[test]
fn every_workflow_parses_as_the_three_blocks_a_workflow_has() {
    // Not a schema check, which would need a YAML parser this crate does not
    // carry and the build container does not have either. It is the structural
    // half a typo takes out: a file that is not a mapping of top-level keys, a
    // job list that is empty, or a tab where the syntax forbids one.
    for (relative, body) in declarative_github_files() {
        assert!(
            !body.contains('\t'),
            "{relative} carries a tab, which this syntax does not allow"
        );

        let top_level: Vec<&str> = body
            .lines()
            .filter(|line| !line.starts_with([' ', '#', '-']) && line.contains(':'))
            .map(|line| line.split(':').next().expect("a key").trim())
            .collect();

        assert!(
            top_level.contains(&"name"),
            "{relative} names itself nowhere"
        );

        if relative.contains("/workflows/") {
            for expected in ["on", "permissions", "jobs"] {
                assert!(
                    top_level.contains(&expected),
                    "{relative} declares no {expected}"
                );
            }
            assert!(
                body.contains("\n    runs-on:") || body.contains("\n    uses:"),
                "{relative} declares no job that runs anywhere"
            );
        } else {
            for expected in ["description", "runs"] {
                assert!(
                    top_level.contains(&expected),
                    "{relative} declares no {expected}"
                );
            }
            assert!(
                body.contains("using: composite"),
                "{relative} is not a composite action"
            );
        }
    }
}

#[test]
fn every_action_a_workflow_uses_is_pinned_to_an_immutable_commit() {
    // M38 section 12.2. A tag is a moving reference and whoever can move it
    // runs inside these jobs; a local action is this repository's own file and
    // is checked for existence instead.
    for (relative, body) in declarative_github_files() {
        for line in body.lines() {
            let Some((_, reference)) = line.trim().split_once("uses: ") else {
                continue;
            };
            let reference = reference.trim();

            if let Some(local) = reference.strip_prefix("./") {
                let path = repository().join(local).join("action.yml");
                assert!(
                    path.is_file(),
                    "{relative} uses {reference}, and there is no action.yml there"
                );
                continue;
            }

            let (_, pin) = reference
                .split_once('@')
                .unwrap_or_else(|| panic!("{relative} uses {reference} with no pin at all"));
            let sha = pin.split_whitespace().next().unwrap_or_default();
            assert!(
                sha.len() == 40 && sha.chars().all(|c| c.is_ascii_hexdigit()),
                "{relative} pins {reference} to something that is not a commit"
            );
            assert!(
                line.contains("# v"),
                "{relative} pins {reference} without saying which version it is"
            );
        }
    }
}

#[test]
fn both_package_builds_go_through_the_one_shared_action() {
    // The reason this action exists: the packages a pull request produces are
    // built by the steps that build the released ones, rather than by a copy of
    // them that drifts a release at a time.
    let action = "uses: ./.github/actions/build-packages";
    for name in ["packages.yml", "release-fermix-desktop.yml"] {
        assert!(
            workflow(name).contains(action),
            "{name} builds packages without the shared action"
        );
    }

    let definition =
        std::fs::read_to_string(repository().join(".github/actions/build-packages/action.yml"))
            .expect("the action reads");
    assert!(
        definition.contains("scripts/build_packages.sh \"$VERSION\" \"$ARCH\" --container"),
        "the shared action builds somewhere other than the build container"
    );
}

#[test]
fn the_packaging_workflow_covers_every_file_that_can_change_a_package() {
    let body = workflow("packages.yml");

    for trigger in ["workflow_dispatch:", "pull_request:"] {
        assert!(
            body.contains(trigger),
            "packages.yml does not run on {trigger}"
        );
    }

    // A change to any of these changes what the packages contain or what they
    // declare, and none of them is covered by a gate that builds no package.
    // A minimum, not the whole list: the gate asserts each of these is watched
    // and says nothing about a workflow watching more. The last four arrived
    // with the single package and each one can now change what the package
    // contains where none of them could before -- a pin bump that did not
    // rebuild the package would be exactly the wrong thing to be quiet about.
    for path in [
        "packaging/**",
        "scripts/build_packages.sh",
        "App/Fermix/Cargo.toml",
        ".github/workflows/packages.yml",
        ".github/actions/build-packages/action.yml",
        "scripts/package_dependencies.py",
        "scripts/check_private_runtime.sh",
        "scripts/check_copyright.sh",
        "engine/PIN.json",
    ] {
        assert!(
            body.contains(&format!("\"{path}\"")),
            "packages.yml does not run when {path} changes"
        );
    }

    for runner in ["ubuntu-24.04\n", "ubuntu-24.04-arm\n"] {
        assert!(
            body.contains(runner),
            "packages.yml builds nothing on {}",
            runner.trim()
        );
    }
    assert!(
        body.contains("fermix-desktop-packages-${{ matrix.arch }}"),
        "packages.yml uploads its artifacts under some other name"
    );
    assert!(
        body.contains("build-id: pr-${{ github.run_id }}"),
        "a package built for review must say which run built it"
    );

    // It signs nothing and publishes nothing, because a release rail that grew
    // into this one would be a second way to publish.
    //
    // The test is on what cosign is asked to DO, not on whether it is present.
    // Installing cosign is now a requirement rather than a smell: the build
    // needs a pinned engine, and `scripts/verify_engine.sh` verifies that
    // artifact's cosign identity before anything is unpacked. Forbidding the
    // tool outright would forbid the verification and leave a workflow that
    // builds from an unverified engine looking cleaner than one that does not.
    for forbidden in ["cosign sign", "cosign attest", "gh release", "environment:"] {
        assert!(
            !body.contains(forbidden),
            "packages.yml carries {forbidden}, and it signs and publishes nothing"
        );
    }

    // And the other half of that: if it carries cosign at all, it is there to
    // verify. A workflow that installed it and never used it would be a signing
    // step half-written.
    if body.contains("cosign") {
        assert!(
            body.contains("verify_engine.sh") || body.contains("cosign verify"),
            "packages.yml installs cosign and verifies nothing with it"
        );
    }
}

#[test]
fn the_release_rail_refuses_a_contract_vendored_from_an_uncommitted_engine() {
    // The release audience of the contract pin. A note is what a developer
    // gets; a refusal is what a release gets.
    assert!(
        workflow("release-fermix-desktop.yml").contains("scripts/verify_contract.sh --release"),
        "a release could ship an application built against bytes nobody else can retrieve"
    );
}

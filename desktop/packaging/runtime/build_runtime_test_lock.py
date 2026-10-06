#!/usr/bin/env python3
"""Hold RUNTIME.lock.json to its rules. Run by build_runtime_test.sh.

The lock file is the runtime's whole definition: what is built, from which bytes,
with which flags, and what may be taken from the host. Each rule below is one a
later edit could break without any build noticing until a user's machine did.
"""

import json
import re
import sys

FIELDS = ("version", "purl", "url", "sha256", "archive", "source_dir",
          "build_system", "license", "options", "patches")
SYSTEMS = {"meson", "autotools", "cmake"}

# Every private component the window needs, by name. One dropped from the lock
# is a library the package would take from the host without saying so.
REQUIRED = {
    "libffi", "pcre2", "expat", "libpng", "libjpeg-turbo", "libtiff", "libwebp",
    "freetype", "fribidi", "pixman", "fontconfig", "glib", "harfbuzz", "cairo",
    "graphene", "pango", "gdk-pixbuf", "webp-pixbuf-loader", "libxml2", "librsvg",
    "libxmlb", "appstream", "wayland", "wayland-protocols", "libepoxy", "gtk",
    "libadwaita", "dconf", "adwaita-icon-theme", "abseil-cpp",
    "webrtc-audio-processing", "gstreamer", "gst-plugins-base", "gst-plugins-good",
    "gst-plugins-bad",
}
# Components the runtime decided against; see README.md.
REFUSED = {"glycin", "orc"}
# Components built from and never shipped: their licence files are listed, and
# marked so.
BUILD_ONLY = {"wayland-protocols"}

HOST_REQUIRED = {
    "libc.so.6", "libm.so.6", "libgcc_s.so.1", "libstdc++.so.6",
    "libGL.so.1", "libEGL.so.1", "libGLESv2.so.2", "libdbus-1.so.3",
    "libX11.so.6", "libxkbcommon.so.0", "libz.so.1", "libyaml-0.so.2",
    "libcurl.so.4", "libpulse.so.0",
}
# The pkg-config package each -sys crate in desktop/Cargo.lock links, or None
# for one that links no library of the runtime. The window links exactly these,
# so they are the roots the shipped tree is pruned from.
SYS_CRATES = {
    "cairo-sys-rs": "cairo", "gdk-pixbuf-sys": "gdk-pixbuf-2.0", "gdk4-sys": "gtk4",
    "gio-sys": "gio-2.0", "glib-sys": "glib-2.0", "gobject-sys": "gobject-2.0",
    "graphene-sys": "graphene-gobject-1.0", "gsk4-sys": "gtk4",
    "gstreamer-app-sys": "gstreamer-app-1.0",
    "gstreamer-audio-sys": "gstreamer-audio-1.0",
    "gstreamer-base-sys": "gstreamer-base-1.0", "gstreamer-sys": "gstreamer-1.0",
    "gtk4-sys": "gtk4", "libadwaita-sys": "libadwaita-1", "pango-sys": "pango",
    "linux-raw-sys": None, "windows-sys": None,
}
# Private libraries, and host ones whose SONAME differs across the matrix or
# that GLib is built without.
HOST_FORBIDDEN = {
    "libgtk-4.so.1", "libadwaita-1.so.0", "libglib-2.0.so.0", "libgio-2.0.so.0",
    "libpango-1.0.so.0", "libcairo.so.2", "libharfbuzz.so.0", "libfontconfig.so.1",
    "libfreetype.so.6", "libwayland-client.so.0", "libgstreamer-1.0.so.0",
    "libxml2.so.2", "libffi.so.8", "libtiff.so.6", "libjpeg.so.8",
    "libselinux.so.1", "libmount.so.1", "libblkid.so.1",
}


def fail(message):
    raise SystemExit("lock: " + message)


def check_header(lock, dockerfile):
    if lock.get("schema_version") != 1:
        fail("schema_version is not 1")
    if lock["prefix"] != "/usr/lib/fermix-desktop":
        fail("the prefix is not the installed one")
    if lock["glibc_floor"] != "2.34":
        fail("the glibc floor moved")
    # Ubuntu 22.04's libstdc++6 12.3.0, the oldest in the matrix.
    if lock["glibcxx_ceiling"] != "3.4.30" or lock["cxxabi_ceiling"] != "1.3.13":
        fail("the libstdc++ ceilings are not Ubuntu 22.04's")
    if not re.fullmatch(r"almalinux@sha256:[0-9a-f]{64}", lock["base_image"]):
        fail("the base image is not AlmaLinux pinned by digest")
    if ("FROM %s\n" % lock["base_image"]) not in dockerfile:
        fail("the Dockerfile is not built FROM the lock file's base image")
    if not isinstance(lock["source_date_epoch"], int):
        fail("source_date_epoch is not an integer")


def check_component(component):
    name = component["name"]
    missing = [field for field in FIELDS if field not in component]
    if missing:
        fail("%s has no %s" % (name, ", ".join(missing)))
    # The coordinate the vulnerability watch queries has to name what is built.
    if component["purl"] != "pkg:generic/%s@%s" % (name, component["version"]):
        fail("%s has a purl that does not match its name and version" % name)
    if component["build_system"] not in SYSTEMS:
        fail("%s has an unknown build system" % name)
    if not re.fullmatch(r"[0-9a-f]{64}", component["sha256"]):
        fail("%s has no sha256" % name)
    if not component["url"].startswith("https://"):
        fail("%s is fetched over something other than https" % name)
    if component["version"] not in component["url"]:
        fail("%s: the url does not name the version" % name)
    if not component["archive"].endswith((".tar.xz", ".tar.gz", ".tar.bz2")):
        fail("%s: the archive is not a tarball" % name)
    if not all(isinstance(component[field], list) for field in ("options", "patches")):
        fail("%s: options and patches are lists" % name)
    check_licence_fields(component)
    if "cargo" in component:
        check_cargo(name, component["cargo"])


def check_licence_fields(component):
    """What runtime-licenses.json reads from a component beyond its licence."""
    name = component["name"]
    if component.get("build_only", True) is not True:
        fail("%s: build_only is true or absent" % name)
    for path in component.get("extra_license_files", []):
        if not is_inner_path(path):
            fail("%s: the licence file %r is not a path inside its source tree" % (name, path))


def is_inner_path(path):
    return (isinstance(path, str) and path != "" and not path.startswith("/")
            and ".." not in path.split("/"))


def check_cargo(name, cargo):
    """The Cargo.lock that pins a Rust component's crates, and the packages and
    features its build compiles, which name the crates compiled in."""
    lock = cargo.get("lock") if isinstance(cargo, dict) else None
    if (not isinstance(lock, str) or not lock.endswith("Cargo.lock")
            or lock.startswith("/") or ".." in lock):
        fail("%s: cargo.lock is not a Cargo.lock inside the source tree" % name)
    packages = cargo.get("packages")
    if not isinstance(packages, dict) or not packages:
        fail("%s: cargo.packages names no package" % name)
    for package, features in packages.items():
        if not (isinstance(features, list)
                and all(isinstance(f, str) for f in features)):
            fail("%s: the features of %s are not a list of names" % (name, package))
    # For a crate whose package carries no licence file: the SPDX ids of the
    # standard texts that stand for it, from license_text_source.
    for crate, ids in cargo.get("standard_license_texts", {}).items():
        if not re.fullmatch(r"[A-Za-z0-9_-]+ [0-9A-Za-z.+-]+", crate):
            fail("%s: standard_license_texts key %r is not 'name version'" % (name, crate))
        if not ids or not all(re.fullmatch(r"[A-Za-z0-9.+-]+", i or "") for i in ids):
            fail("%s: the standard texts of %s are not SPDX ids" % (name, crate))


def by_name(lock):
    return {c["name"]: c for c in lock["components"]}


def check_set(lock):
    names = [c["name"] for c in lock["components"]]
    if len(names) != len(set(names)):
        fail("a component is listed twice")
    missing = REQUIRED - set(names)
    if missing:
        fail("missing: %s" % ", ".join(sorted(missing)))
    present = REFUSED & set(names)
    if present:
        fail("components the runtime decided against: %s" % ", ".join(sorted(present)))
    build_only = {c["name"] for c in lock["components"] if c.get("build_only")}
    if build_only != BUILD_ONLY:
        fail("the build-only components are %s, not %s" % (sorted(build_only), sorted(BUILD_ONLY)))
    # The lock order is the build order: each needs what comes before it.
    order = names.index
    for before, after in (
            ("glib", "gtk"), ("gtk", "libadwaita"), ("gtk", "adwaita-icon-theme"),
            ("appstream", "libadwaita"), ("abseil-cpp", "webrtc-audio-processing"),
            ("webrtc-audio-processing", "gst-plugins-bad"),
            ("gstreamer", "gst-plugins-base"), ("gst-plugins-base", "gst-plugins-good"),
            ("gdk-pixbuf", "webp-pixbuf-loader"), ("wayland", "gtk"),
            ("gdk-pixbuf", "librsvg"), ("libxml2", "librsvg"), ("pango", "librsvg")):
        if order(before) > order(after):
            fail("%s is built after %s, which needs it" % (before, after))


def check_versions(lock):
    pinned = {name: c["version"] for name, c in by_name(lock).items()}
    for name, series in (("gtk", "4.22."), ("libadwaita", "1.9."), ("glib", "2.88."),
                         ("adwaita-icon-theme", "50.")):
        if not pinned[name].startswith(series):
            fail("%s %s is not inside %sx" % (name, pinned[name], series))
    modules = ("gstreamer", "gst-plugins-base", "gst-plugins-good", "gst-plugins-bad")
    gst = {pinned[n] for n in modules}
    if len(gst) != 1 or not next(iter(gst)).startswith("1.26."):
        fail("the four GStreamer modules are not one 1.26.x release: %s" % sorted(gst))


def require_flags(component, flags):
    missing = [flag for flag in flags if flag not in component["options"]]
    if missing:
        fail("%s is not built with %s" % (component["name"], " ".join(missing)))


def check_flags(lock):
    c = by_name(lock)
    require_flags(c["glib"], ["-Dselinux=disabled", "-Dlibmount=disabled"])
    require_flags(c["gtk"], ["-Dx11-backend=true", "-Dwayland-backend=true",
                             "-Dvulkan=disabled", "-Dmedia-gstreamer=disabled",
                             "-Dprint-cups=disabled", "-Dintrospection=disabled",
                             "-Daccesskit=disabled"])
    # PNG, JPEG, GIF and WebP through gdk-pixbuf, and nothing it would hand to
    # glycin. Chats send GIFs.
    require_flags(c["gdk-pixbuf"], ["-Dpng=enabled", "-Djpeg=enabled",
                                    "-Dgif=enabled", "-Dtiff=disabled",
                                    "-Dothers=disabled", "-Dbuiltin_loaders=none"])
    require_flags(c["librsvg"], ["-Dpixbuf-loader=enabled", "-Drsvg-convert=disabled",
                                 "-Davif=disabled"])
    check_librsvg_cargo(c["librsvg"])
    # FreeType's LICENSE.TXT only points at the two licences it offers.
    if c["freetype"].get("extra_license_files") != ["docs/FTL.TXT", "docs/GPLv2.TXT"]:
        fail("freetype does not name docs/FTL.TXT and docs/GPLv2.TXT as its licence files")
    require_flags(c["libtiff"], ["--disable-cxx"])
    require_flags(c["harfbuzz"], ["-Dsubset=enabled", "-Dwith_libstdcxx=false"])
    require_flags(c["abseil-cpp"], ["-DBUILD_SHARED_LIBS=OFF",
                                    "-DCMAKE_POSITION_INDEPENDENT_CODE=ON"])
    # Nothing in a GStreamer module is built unless it is named here.
    enables = {
        "gstreamer": ["-Dtools=enabled"],
        "gst-plugins-base": ["-Dapp=enabled", "-Daudioconvert=enabled",
                             "-Daudioresample=enabled", "-Daudiotestsrc=enabled"],
        "gst-plugins-good": ["-Dpulse=enabled", "-Dlevel=enabled"],
        "gst-plugins-bad": ["-Dwebrtcdsp=enabled"],
    }
    for name, flags in enables.items():
        require_flags(c[name], ["-Dauto_features=disabled"] + flags)
        extra = [f for f in c[name]["options"]
                 if f.endswith("=enabled") and f not in flags]
        if extra:
            fail("%s enables more than the window needs: %s" % (name, " ".join(extra)))


def check_librsvg_cargo(component):
    """librsvg's meson options decide which Rust packages it builds and with
    which features; the lock's cargo entry has to say the same, or the crate
    list written from it names crates that were not compiled in, or misses some."""
    cargo = component.get("cargo")
    if not cargo or cargo.get("lock") != "Cargo.lock":
        fail("librsvg does not name the Cargo.lock that pins its crates")
    options = component["options"]
    expected = {"librsvg-c": ["pixbuf"] if "-Dpixbuf=enabled" in options else []}
    if "-Dpixbuf-loader=enabled" in options:
        expected["pixbufloader-svg"] = []
    if "-Davif=enabled" in options:
        expected["librsvg-c"].append("avif")
    if cargo["packages"] != expected:
        fail("librsvg's cargo packages %s are not what its options build: %s"
             % (cargo["packages"], expected))


def check_application(lock, cargo_lock_text):
    """application_packages is what the window's -sys crates link."""
    names = set(re.findall(r'^name = "([^"]+-sys(?:-rs)?)"$', cargo_lock_text, re.M))
    unknown = names - set(SYS_CRATES)
    if unknown:
        fail("desktop/Cargo.lock has %s, which SYS_CRATES does not map"
             % ", ".join(sorted(unknown)))
    linked = {SYS_CRATES[n] for n in names} - {None}
    declared = lock.get("application_packages")
    if not isinstance(declared, list) or len(declared) != len(set(declared)):
        fail("application_packages is not a list of distinct names")
    if set(declared) != linked:
        fail("application_packages %s is not what the window links: %s"
             % (sorted(declared), sorted(linked)))


def check_text_source(lock):
    """SPDX's license-list-data, pinned like a component and never built."""
    source = lock.get("license_text_source")
    if not isinstance(source, dict):
        fail("there is no license_text_source")
    for field in ("name", "version", "url", "sha256", "archive", "source_dir", "text_dir"):
        if not isinstance(source.get(field), str) or not source[field]:
            fail("license_text_source has no %s" % field)
    if source["name"] != "license-list-data" or not source["url"].startswith(
            "https://github.com/spdx/license-list-data/"):
        fail("license_text_source is not SPDX's license-list-data")
    if source["version"] not in source["url"] or not re.fullmatch(r"[0-9a-f]{64}",
                                                                  source["sha256"]):
        fail("license_text_source is not pinned by version and sha256")
    if not source["archive"].endswith(".tar.gz") or not is_inner_path(source["text_dir"]):
        fail("license_text_source is not a tarball with a text directory inside it")
    if source["name"] in {c["name"] for c in lock["components"]}:
        fail("license_text_source is listed as a component, which would build it")


def check_hosts(lock):
    hosts = lock["host_libraries"]
    if len(hosts) != len(set(hosts)):
        fail("a host library is listed twice")
    missing = HOST_REQUIRED - set(hosts)
    if missing:
        fail("not on the host list: %s" % ", ".join(sorted(missing)))
    forbidden = HOST_FORBIDDEN & set(hosts)
    if forbidden:
        fail("on the host list and must not be: %s" % ", ".join(sorted(forbidden)))
    # libpulsecommon's SONAME carries the PulseAudio version, so no one name is on
    # every host.
    if any(name.startswith("libpulsecommon") for name in hosts):
        fail("libpulsecommon is on the host list; its SONAME differs per distribution")


def main(argv):
    if len(argv) != 3:
        fail("usage: build_runtime_test_lock.py <lock file> <Dockerfile.runtime>"
             " <desktop/Cargo.lock>")
    with open(argv[0], encoding="utf-8") as handle:
        lock = json.load(handle)
    with open(argv[1], encoding="utf-8") as handle:
        dockerfile = handle.read()
    with open(argv[2], encoding="utf-8") as handle:
        cargo_lock_text = handle.read()
    check_header(lock, dockerfile)
    for component in lock["components"]:
        check_component(component)
    check_set(lock)
    check_versions(lock)
    check_flags(lock)
    check_hosts(lock)
    check_text_source(lock)
    check_application(lock, cargo_lock_text)
    print("  ok: %d components, every digest, every required flag, the host list"
          % len(lock["components"]))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))

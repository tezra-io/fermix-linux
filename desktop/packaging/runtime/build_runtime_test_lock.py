#!/usr/bin/env python3
"""Hold RUNTIME.lock.json to its rules. Run by build_runtime_test.sh.

The lock file is the runtime's whole definition: what is built, from which bytes,
with which flags, and what may be taken from the host. Each rule below is one a
later edit could break without any build noticing until a user's machine did.
"""

import hashlib
import json
import os
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
# Components that compile in code or ship data under licences beyond their main
# one: the expression, which names every licence of what ships; the files beyond
# the top-level ones that hold those licences' texts (extra_license_files, and
# the file of each excerpt license_excerpts takes); the file each LicenseRef's
# text is in; and the licences whose standard text stands in, where no file of
# the component holds it. README.md gives the evidence for each, against
# Debian's copyright file for the same package.
UNICODE = ["Unicode-3.0"]
LGPL_2_0 = ["LGPL-2.0-or-later"]
# GStreamer's shipped sources are nearly all "version 2 of the License, or (at
# your option) any later version"; its COPYING is the LGPL 2.1.
GSTREAMER = {"license": "LGPL-2.1-or-later AND LGPL-2.0-or-later", "files": [],
             "refs": {}, "standard": LGPL_2_0}
AUDITED = {
    "libffi": {"license": "MIT AND CC0-1.0", "files": ["src/dlmalloc.c"], "refs": {},
               "standard": ["CC0-1.0"]},
    "freetype": {"license": "(FTL OR GPL-2.0-or-later) AND MIT AND MIT-open-group",
                 "files": ["docs/FTL.TXT", "docs/GPLv2.TXT", "src/base/fthash.c",
                           "src/bdf/README", "src/pcf/README", "src/pcf/pcfutil.c"],
                 "refs": {}, "standard": []},
    "glib": {
        "license": "LGPL-2.1-or-later AND (LGPL-2.1-or-later OR AFL-2.0) AND LGPL-2.0-or-later"
                   " AND MIT AND bzip2-1.0.6 AND LicenseRef-glib-gbsearcharray"
                   " AND LicenseRef-glib-gchecksum AND Unicode-3.0",
        "files": ["glib/gbsearcharray.h", "glib/gchecksum.c", "glib/gchecksum.c",
                  "glib/gutilsprivate.c", "glib/valgrind.h"],
        "refs": {"LicenseRef-glib-gbsearcharray": "glib/gbsearcharray.h.notice",
                 "LicenseRef-glib-gchecksum": "glib/gchecksum.c.notice"},
        "standard": LGPL_2_0 + UNICODE},
    "gtk": {"license": "LGPL-2.1-or-later AND LGPL-2.0-or-later AND Apache-2.0 AND MIT"
                       " AND MIT-open-group AND HPND-sell-variant AND TCL AND CC0-1.0"
                       " AND Unicode-3.0",
            "files": ["gtk/roaring/COPYING", "gtk/timsort/COPYING",
                      "gdk/wayland/protocol/xx-session-management-v1.xml",
                      "gdk/x11/gdkasync.c", "gdk/x11/gdkxftdefaults.c",
                      "gdk/x11/xsettings-client.c", "gtk/gtktextbtree.c",
                      "gtk/inspector/css-node-tree.c", "gtk/inspector/logs.c",
                      "gtk/inspector/window.c"],
            "refs": {}, "standard": ["CC0-1.0", "LGPL-2.1-or-later"] + UNICODE},
    "harfbuzz": {"license": "MIT-Modern-Variant AND MIT AND ISC AND Unicode-3.0",
                 "files": ["src/ms-use/COPYING", "src/hb-ucd.cc"], "refs": {},
                 "standard": UNICODE},
    "pcre2": {"license": "BSD-3-Clause WITH PCRE2-exception AND BSD-2-Clause AND Unicode-3.0",
              "files": ["deps/sljit/LICENSE"], "refs": {}, "standard": UNICODE},
    "webrtc-audio-processing": {
        "license": "BSD-3-Clause AND BSD-2-Clause AND LicenseRef-webrtc-ooura"
                   " AND LicenseRef-webrtc-spl-sqrt-floor AND LicenseRef-webrtc-pffft",
        "files": ["webrtc/LICENSE", "webrtc/PATENTS",
                  "webrtc/common_audio/third_party/ooura/LICENSE",
                  "webrtc/common_audio/third_party/spl_sqrt_floor/LICENSE",
                  "webrtc/third_party/pffft/LICENSE", "webrtc/third_party/rnnoise/COPYING",
                  "webrtc/third_party/rnnoise/src/rnn_activations.h"],
        "refs": {"LicenseRef-webrtc-ooura": "webrtc/common_audio/third_party/ooura/LICENSE",
                 "LicenseRef-webrtc-spl-sqrt-floor":
                     "webrtc/common_audio/third_party/spl_sqrt_floor/LICENSE",
                 "LicenseRef-webrtc-pffft": "webrtc/third_party/pffft/LICENSE"},
        "standard": []},
    "fribidi": {"license": "LGPL-2.1-or-later AND Unicode-3.0", "files": [], "refs": {},
                "standard": UNICODE},
    "fontconfig": {
        "license": "HPND-sell-variant AND MIT-Modern-Variant AND MIT"
                   " AND LicenseRef-fontconfig-fcmd5 AND LicenseRef-fontconfig-ftglue"
                   " AND Unicode-3.0",
        "files": ["src/fcmd5.h", "src/ftglue.c"],
        "refs": {"LicenseRef-fontconfig-fcmd5": "src/fcmd5.h.notice",
                 "LicenseRef-fontconfig-ftglue": "src/ftglue.c.notice"},
        "standard": UNICODE},
    "cairo": {"license": "(LGPL-2.1-only OR MPL-1.1) AND HPND-sell-variant",
              "files": ["src/cairo-pattern.c"], "refs": {}, "standard": []},
    "pango": {"license": "LGPL-2.0-or-later AND LGPL-2.1-or-later AND BSD-3-Clause AND ICU"
                         " AND TCL AND Unicode-3.0",
              "files": ["pango/emoji_presentation_scanner.c", "pango/pango-color.c",
                        "pango/pango-script.c"],
              "refs": {}, "standard": ["BSD-3-Clause", "LGPL-2.1-or-later"] + UNICODE},
    "gdk-pixbuf": {"license": "LGPL-2.1-or-later AND LGPL-2.0-or-later", "files": [],
                   "refs": {}, "standard": LGPL_2_0},
    # Its LICENSE.LGPL-2 is the notice, not the licence.
    "webp-pixbuf-loader": {"license": "LGPL-2.0-or-later", "files": [], "refs": {},
                           "standard": LGPL_2_0},
    # LICENSE.md names the zlib License of the SIMD code by a link only.
    "libjpeg-turbo": {"license": "IJG AND BSD-3-Clause AND Zlib",
                      "files": ["README.ijg", "simd/jsimd.c", "simd/nasm/jsimdext.inc"],
                      "refs": {}, "standard": []},
    "libxml2": {"license": "MIT AND ISC AND Unicode-3.0", "files": ["dict.c", "list.c"],
                "refs": {}, "standard": UNICODE},
    "appstream": {"license": "LGPL-2.1-or-later AND FSFAP", "files": [], "refs": {},
                  "standard": ["FSFAP"]},
    "wayland-protocols": {
        "license": "MIT AND HPND-sell-variant",
        "files": ["unstable/pointer-gestures/pointer-gestures-unstable-v1.xml",
                  "unstable/text-input/text-input-unstable-v3.xml"],
        "refs": {}, "standard": []},
    "dconf": {"license": "LGPL-2.1-or-later AND LGPL-2.0-or-later", "files": [],
              "refs": {}, "standard": LGPL_2_0},
    "adwaita-icon-theme": {"license": "(CC-BY-SA-3.0 OR LGPL-3.0-only) AND CC-BY-SA-4.0",
                           "files": [], "refs": {}, "standard": ["CC-BY-SA-4.0"]},
    # gst/gsturi.c compiles FreeBSD's strcasestr: meson never defines
    # HAVE_STRCASESTR.
    "gstreamer": dict(GSTREAMER, license=GSTREAMER["license"] + " AND BSD-3-Clause",
                      files=["gst/gsturi.c"]),
    "gst-plugins-base": GSTREAMER,
    "gst-plugins-good": GSTREAMER,
    "gst-plugins-bad": GSTREAMER,
}
# The standard texts and the licence and exception lists: raw files of SPDX's
# license-list-data at one release tag.
LICENSE_LIST_VERSION = "3.29.0"
LICENSE_LIST_URL = "https://raw.githubusercontent.com/spdx/license-list-data/v%s/%s"
LICENSE_LISTS = {"licenses": "json/licenses.json", "exceptions": "json/exceptions.json"}

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


def check_component(component, spdx):
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
    check_licence_fields(component, spdx)
    if "cargo" in component:
        check_cargo(name, component["cargo"])


def check_licence_fields(component, spdx):
    """The licence, written as SPDX writes it, and what runtime-licenses.json
    reads from a component beyond it. spdx is write_crates.py."""
    name = component["name"]
    if spdx.spdx_licence(component["license"]) != component["license"]:
        fail("%s: %r is not an SPDX expression" % (name, component["license"]))
    if component.get("build_only", True) is not True:
        fail("%s: build_only is true or absent" % name)
    for path in component.get("extra_license_files", []):
        if not is_inner_path(path):
            fail("%s: the licence file %r is not a path inside its source tree" % (name, path))
    excerpts = component.get("license_excerpts", [])
    if not isinstance(excerpts, list) or not all(isinstance(e, dict) for e in excerpts):
        fail("%s: license_excerpts is not a list of excerpts" % name)
    whole = {excerpt.get("path") for excerpt in excerpts} & set(
        component.get("extra_license_files", []))
    if whole:
        fail("%s: %s is named whole and as an excerpt" % (name, ", ".join(sorted(whole))))
    for excerpt in excerpts:
        check_excerpt(name, excerpt)
    check_excerpt_order(name, excerpts)
    check_texts_of(component, spdx)


def check_texts_of(component, spdx):
    """Where the text of a licence comes from when no top-level file of the
    component holds it: license_refs names the file for each LicenseRef, one
    that extra_license_files or license_excerpts names, and
    standard_license_texts the licences whose standard text stands in."""
    name = component["name"]
    licences = {term for term, is_exception in spdx.spdx_terms(component["license"])
                if not is_exception}
    named = {term for term in licences if term.startswith("LicenseRef-")}
    refs = component.get("license_refs", {})
    if not isinstance(refs, dict) or set(refs) != named:
        fail("%s: its licence names %s, and license_refs maps %s"
             % (name, sorted(named), sorted(refs)))
    own = set(component.get("extra_license_files", [])) | {
        excerpt["path"] + ".notice" for excerpt in component.get("license_excerpts", [])}
    for ref, path in sorted(refs.items()):
        if path not in own:
            fail("%s: %s maps to %r, which is not a file extra_license_files or"
                 " license_excerpts names" % (name, ref, path))
    standard = component.get("standard_license_texts", [])
    if (not isinstance(standard, list) or len(standard) != len(set(standard))
            or not set(standard) <= licences - named):
        fail("%s: standard_license_texts %s is not a list of licences its licence names"
             % (name, standard))


def check_excerpt(name, excerpt):
    """A notice inside a source file: its path, its first and last lines, and
    the sha256 of those lines."""
    if set(excerpt) != {"path", "lines", "sha256"} or not is_inner_path(excerpt["path"]):
        fail("%s: the excerpt %r is not {path, lines, sha256} inside its source tree"
             % (name, excerpt))
    lines = excerpt["lines"]
    if not (isinstance(lines, list) and len(lines) == 2
            and all(type(n) is int for n in lines) and 1 <= lines[0] <= lines[1]):
        fail("%s: the lines of %s are not [first, last]" % (name, excerpt["path"]))
    if not re.fullmatch(r"[0-9a-f]{64}", str(excerpt["sha256"])):
        fail("%s: the excerpt of %s has no sha256" % (name, excerpt["path"]))


def check_excerpt_order(name, excerpts):
    """The excerpts of one file make one notice, in the lock's order: they come
    in the file's order and share no line."""
    last = {}
    for excerpt in excerpts:
        path, (first, end) = excerpt["path"], excerpt["lines"]
        if first <= last.get(path, 0):
            fail("%s: the excerpts of %s overlap or are out of order" % (name, path))
        last[path] = end


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
    # standard texts that stand for it, from license_text_sources.
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


def check_audited(lock):
    """The audited components keep the expression, the licence files and the
    texts the audit found; an edit that drops a licence of what ships fails here."""
    components = by_name(lock)
    for name, audit in AUDITED.items():
        component = components[name]
        if component["license"] != audit["license"]:
            fail("%s is licensed %r, and the audit found %r"
                 % (name, component["license"], audit["license"]))
        # A file is named once for each excerpt of it.
        named = sorted(component.get("extra_license_files", []) + [
            excerpt["path"] for excerpt in component.get("license_excerpts", [])])
        found = (named, component.get("license_refs", {}),
                 component.get("standard_license_texts", []))
        audited = (sorted(audit["files"]), audit["refs"], audit["standard"])
        if found != audited:
            fail("%s names %s, and the audit found %s" % (name, found, audited))


def check_refs_unique(lock):
    """No LicenseRef is mapped by two components: each has one text."""
    owners = {}
    for component in lock["components"]:
        for ref in component.get("license_refs", {}):
            if ref in owners:
                fail("%s is mapped by both %s and %s" % (ref, owners[ref], component["name"]))
            owners[ref] = component["name"]


def check_text_source(source):
    """One standard text: a raw file of license-list-data at its release tag,
    pinned by sha256, under a name no component archive takes."""
    if not isinstance(source, dict) or set(source) != {"spdx_id", "url", "sha256", "file"}:
        fail("the text source %r is not {spdx_id, url, sha256, file}" % (source,))
    spdx_id = source["spdx_id"]
    if source["url"] != LICENSE_LIST_URL % (LICENSE_LIST_VERSION, "text/%s.txt" % spdx_id):
        fail("the text of %s is not license-list-data v%s's" % (spdx_id, LICENSE_LIST_VERSION))
    if source["file"] != "spdx-license-list-%s-%s.txt" % (LICENSE_LIST_VERSION, spdx_id):
        fail("the text of %s is not kept as spdx-license-list-%s-%s.txt"
             % (spdx_id, LICENSE_LIST_VERSION, spdx_id))
    if not re.fullmatch(r"[0-9a-f]{64}", str(source["sha256"])):
        fail("the text of %s is not pinned by sha256" % spdx_id)


def check_text_sources(lock):
    """The standard texts are exactly the ones crates and components name."""
    sources = lock.get("license_text_sources")
    if not isinstance(sources, list) or not sources:
        fail("there are no license_text_sources")
    for source in sources:
        check_text_source(source)
    pinned = [source["spdx_id"] for source in sources]
    if len(pinned) != len(set(pinned)):
        fail("a standard text is pinned twice")
    named = {spdx_id for component in lock["components"]
             for ids in component.get("cargo", {}).get("standard_license_texts", {}).values()
             for spdx_id in ids}
    named |= {spdx_id for component in lock["components"]
              for spdx_id in component.get("standard_license_texts", [])}
    if set(pinned) != named:
        fail("the pinned texts %s are not the ones crates and components name, %s"
             % (sorted(pinned), sorted(named)))


def check_license_list(lock):
    """SPDX's licence and exception lists, raw files at the tag the texts come
    from, pinned by sha256."""
    lists = lock.get("license_list")
    if not isinstance(lists, dict) or set(lists) != set(LICENSE_LISTS):
        fail("license_list is not {licenses, exceptions}")
    for role, path in LICENSE_LISTS.items():
        source = lists[role]
        if not isinstance(source, dict) or set(source) != {"url", "sha256", "file"}:
            fail("license_list's %s is not {url, sha256, file}" % role)
        if source["url"] != LICENSE_LIST_URL % (LICENSE_LIST_VERSION, path):
            fail("license_list's %s is not license-list-data v%s's %s"
                 % (role, LICENSE_LIST_VERSION, path))
        if source["file"] != "spdx-license-list-%s-%s.json" % (LICENSE_LIST_VERSION, role):
            fail("license_list's %s is not kept as spdx-license-list-%s-%s.json"
                 % (role, LICENSE_LIST_VERSION, role))
        if not re.fullmatch(r"[0-9a-f]{64}", str(source["sha256"])):
            fail("license_list's %s is not pinned by sha256" % role)


def load_listed(lock, sources, spdx):
    """SPDX's two lists from the source cache, each held to the lock's digest."""
    paths = []
    for role in ("licenses", "exceptions"):
        source = lock["license_list"][role]
        path = os.path.join(sources, source["file"])
        if not os.path.isfile(path):
            fail("%s is not in %s; build_runtime.sh --fetch fetches every pinned source"
                 % (source["file"], sources))
        with open(path, "rb") as handle:
            digest = hashlib.sha256(handle.read()).hexdigest()
        if digest != source["sha256"]:
            fail("%s has sha256 %s, and the lock says %s" % (path, digest, source["sha256"]))
        paths.append(path)
    return spdx.spdx_list(*paths)


def check_listed(lock, spdx, listed):
    """Every licence and exception the lock names is on SPDX's lists, and none is
    deprecated there: each component's, and each standard text's."""
    for component in lock["components"]:
        problems = spdx.unlisted(component["license"], listed)
        if problems:
            fail("%s: %s" % (component["name"], "; ".join(problems)))
    for source in lock["license_text_sources"]:
        problems = spdx.unlisted(source["spdx_id"], listed)
        if problems:
            fail("the standard text of %s: %s" % (source["spdx_id"], "; ".join(problems)))


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
    if len(argv) != 4:
        fail("usage: build_runtime_test_lock.py <lock file> <Dockerfile.runtime>"
             " <desktop/Cargo.lock> <source cache>")
    with open(argv[0], encoding="utf-8") as handle:
        lock = json.load(handle)
    with open(argv[1], encoding="utf-8") as handle:
        dockerfile = handle.read()
    with open(argv[2], encoding="utf-8") as handle:
        cargo_lock_text = handle.read()
    # write_crates.py's SPDX parser and list check, which read the crates' too.
    sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
    import write_crates
    check_header(lock, dockerfile)
    for component in lock["components"]:
        check_component(component, write_crates)
    check_set(lock)
    check_refs_unique(lock)
    check_versions(lock)
    check_flags(lock)
    check_hosts(lock)
    check_audited(lock)
    check_text_sources(lock)
    check_license_list(lock)
    check_listed(lock, write_crates, load_listed(lock, argv[3], write_crates))
    check_application(lock, cargo_lock_text)
    print("  ok: %d components, every digest, every required flag, the host list,"
          " every licence expression, LicenseRef and standard text, each id on SPDX's"
          " lists" % len(lock["components"]))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))

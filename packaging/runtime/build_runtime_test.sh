#!/usr/bin/env bash
#
# Exercise build_runtime.sh's refusals, and hold the lock file to its own rules,
# without compiling anything.
#
# The gate it wraps takes an hour; its argument handling, its lock file and the
# claims its Dockerfile makes take milliseconds, and those are the parts that
# break silently. Docker is never invoked here: the script is driven with an
# empty PATH or against a throwaway copy of the tree.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
RUNTIME_DIR="$ROOT_DIR/packaging/runtime"
SCRIPT="$RUNTIME_DIR/build_runtime.sh"
SMOKE="$RUNTIME_DIR/smoke_runtime.sh"
SOURCES="$RUNTIME_DIR/package_sources.sh"
LOCK="$RUNTIME_DIR/RUNTIME.lock.json"
DOCKERFILE="$ROOT_DIR/packaging/docker/Dockerfile.runtime"

WORK="$(mktemp -d "${TMPDIR:-/tmp}/build-runtime-test.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

fail() {
  echo "build_runtime_test: $*" >&2
  exit 1
}

echo "build_runtime_test: the scripts parse"
bash -n "$SCRIPT" || fail "build_runtime.sh does not parse"
bash -n "$SMOKE" || fail "smoke_runtime.sh does not parse"
bash -n "$SOURCES" || fail "package_sources.sh does not parse"
# Compiled in memory rather than with `python3 -m py_compile`, which would leave
# a __pycache__ directory in a tree three other slices are editing.
python3 -c 'import sys
for name in sys.argv[1:]:
    with open(name, encoding="utf-8") as handle:
        compile(handle.read(), name, "exec")' \
  "$RUNTIME_DIR/write_manifest.py" "$RUNTIME_DIR/compare_manifest.py" "$RUNTIME_DIR/check_options.py" \
  || fail "a manifest helper does not parse"
echo "  ok: shell and python syntax"

echo "build_runtime_test: refusals"

if "$SCRIPT" --unknown-argument >/dev/null 2>&1; then
  fail "an unknown argument was accepted"
fi
echo "  refused: an unknown argument"

if "$SCRIPT" >/dev/null 2>&1; then
  fail "a run with no mode was accepted"
fi
echo "  refused: no mode"

if "$SMOKE" --unknown-argument >/dev/null 2>&1; then
  fail "smoke_runtime.sh accepted an unknown argument"
fi
echo "  refused: an unknown argument to the smoke"

if "$SOURCES" >/dev/null 2>&1; then
  fail "package_sources.sh accepted a run with no version"
fi
echo "  refused: a source archive with no version"

for bad in 1.2.3-1 1.2.3:4 1.2 nightly; do
  if "$SOURCES" "$bad" >/dev/null 2>&1; then
    fail "package_sources.sh accepted the version '$bad'"
  fi
done
echo "  refused: a Debian revision, an rpm epoch and two malformed versions"

if "$SOURCES" 1.2.3 --unknown-argument >/dev/null 2>&1; then
  fail "package_sources.sh accepted an unknown argument"
fi
echo "  refused: an unknown argument to the source archive"

# Every case below passes --no-fetch, both because that is the offline contract
# worth testing and because a gate that runs in seconds must not download 129 MB.
if "$SOURCES" 1.2.3 --no-fetch --sources "$WORK/missing-cache" --out "$WORK/out" >/dev/null 2>&1; then
  fail "package_sources.sh --no-fetch accepted a cache directory that does not exist"
fi
mkdir -p "$WORK/empty-cache"
if "$SOURCES" 1.2.3 --no-fetch --sources "$WORK/empty-cache" --out "$WORK/out" >/dev/null 2>&1; then
  fail "package_sources.sh --no-fetch accepted an empty download cache"
fi
echo "  refused: --no-fetch with none of the locked tarballs cached"

# The one failure a source archive must never pass, because it looks like
# compliance: files of the right names whose bytes are not what was built.
mkdir -p "$WORK/wrong-cache"
python3 - "$LOCK" "$WORK/wrong-cache" <<'PY'
import json
import os
import sys

lock = json.load(open(sys.argv[1], encoding="utf-8"))
for component in lock["components"]:
    path = os.path.join(sys.argv[2], component["archive"])
    with open(path, "w", encoding="utf-8") as handle:
        handle.write("not the locked tarball\n")
PY
if "$SOURCES" 1.2.3 --no-fetch --sources "$WORK/wrong-cache" --out "$WORK/out" >/dev/null 2>&1; then
  fail "package_sources.sh accepted tarballs that are not the locked ones"
fi
echo "  refused: cached tarballs whose digests are not the locked ones"

mkdir -p "$WORK/empty-bin"
if PATH="$WORK/empty-bin" "$SCRIPT" --container >/dev/null 2>&1; then
  fail "a host without docker was accepted"
fi
echo "  refused: a host with no container runtime"

# A tree whose lock file has been taken away.
mkdir -p "$WORK/no-lock/packaging/runtime/patches" "$WORK/no-lock/packaging/docker"
cp "$SCRIPT" "$WORK/no-lock/packaging/runtime/"
# fetch_source.sh comes too, or the refusal below would be "the library is
# missing" wearing the costume of "the lock file is missing".
cp "$RUNTIME_DIR/fetch_source.sh" "$WORK/no-lock/packaging/runtime/"
cp "$DOCKERFILE" "$WORK/no-lock/packaging/docker/"
if bash "$WORK/no-lock/packaging/runtime/build_runtime.sh" --container >/dev/null 2>&1; then
  fail "a tree with no lock file was accepted"
fi
echo "  refused: a tree with no lock file"

# --verify with no manifest to verify against.
if FERMIX_RUNTIME_OUT="$WORK/empty-out" "$SCRIPT" --verify >/dev/null 2>&1; then
  fail "--verify was accepted with no manifest"
fi
echo "  refused: --verify with nothing to verify against"

echo "build_runtime_test: the shared fetcher"
# Driven over file:// so the gate never touches the network. The lock file is
# still held to https further down; this is about what the fetch does when a
# server answers wrongly, or not at all.
(
  fail() { echo "fetch: $*" >&2; exit 1; }
  log() { :; }
  FETCH_MAX_ATTEMPTS=2
  FETCH_RETRY_SECONDS=0
  # shellcheck source=packaging/runtime/fetch_source.sh
  . "$RUNTIME_DIR/fetch_source.sh"

  served="$WORK/served.tar.gz"
  echo "the locked bytes" > "$served"
  digest="$(sha256sum "$served" | cut -d' ' -f1)"

  fetch_source served "file://$served" "$WORK/got.tar.gz" "$digest"
  cmp -s "$served" "$WORK/got.tar.gz" || exit 1

  # Each refusal runs in its own subshell, because a refusal is fail(), and
  # fail() exits: calling it directly in an `if` condition would take this whole
  # block down with it and report a passing refusal as a broken fetcher.
  #
  # Wrong bytes must not land at the destination, and must leave no partial.
  if ( fetch_source wrong "file://$served" "$WORK/wrong.tar.gz" \
      "0000000000000000000000000000000000000000000000000000000000000000" ) 2>/dev/null; then
    exit 1
  fi
  [ ! -e "$WORK/wrong.tar.gz" ] || exit 1
  [ ! -e "$WORK/wrong.tar.gz.partial" ] || exit 1

  # A URL that never answers exhausts its bounded attempts and refuses.
  if ( fetch_source missing "file://$WORK/not-here.tar.gz" "$WORK/missing.tar.gz" \
      "$digest" ) 2>/dev/null; then
    exit 1
  fi
  [ ! -e "$WORK/missing.tar.gz" ] || exit 1

  # A scheme that is neither https nor file is refused before curl runs.
  if ( fetch_source insecure "http://example.invalid/x.tar.gz" "$WORK/http.tar.gz" \
      "$digest" ) 2>/dev/null; then
    exit 1
  fi

  # A cached file is taken without fetching, but only if its bytes are right:
  # the URL below does not exist, so a fetch would fail.
  ensure_source cached "file://$WORK/not-here.tar.gz" "$WORK/got.tar.gz" "$digest"
  echo "tampered" > "$WORK/got.tar.gz"
  if ( ensure_source cached "file://$WORK/not-here.tar.gz" "$WORK/got.tar.gz" "$digest" ) 2>/dev/null; then
    exit 1
  fi
) || fail "the shared fetcher does not hold"
echo "  ok: fetches, refuses a wrong digest, exhausts bounded retries, refuses http"
echo "  ok: a cached tarball is re-checked, and a tampered one refused"

echo "build_runtime_test: the lock file"

command -v python3 >/dev/null 2>&1 || fail "python3 is needed to read the lock file"
python3 - "$LOCK" <<'PY' || fail "the lock file does not hold"
import json
import re
import sys

lock = json.load(open(sys.argv[1], encoding="utf-8"))

if lock["prefix"] != "/usr/lib/fermix-desktop":
    raise SystemExit("the lock file names a prefix that is not the installed one")
if lock["glibc_floor"] != "2.34":
    raise SystemExit("the glibc floor moved")
if not lock["base_image"].startswith("almalinux@sha256:"):
    raise SystemExit("the base image is not pinned by digest")

names = [c["name"] for c in lock["components"]]
if len(names) != len(set(names)):
    raise SystemExit("a component is listed twice")

systems = {"meson", "autotools", "cmake"}
for component in lock["components"]:
    name = component["name"]
    for field in ("version", "purl", "url", "sha256", "archive", "source_dir",
                  "build_system", "license", "options", "patches"):
        if field not in component:
            raise SystemExit("%s has no %s" % (name, field))
    # The coordinate the vulnerability watch queries. It has to agree with the
    # version beside it, or the watch asks about a release nobody built.
    if component["purl"] != "pkg:generic/%s@%s" % (name, component["version"]):
        raise SystemExit("%s has a purl that does not match its name and version" % name)
    if component["build_system"] not in systems:
        raise SystemExit("%s has an unknown build system" % name)
    if not re.fullmatch(r"[0-9a-f]{64}", component["sha256"]):
        raise SystemExit("%s has no sha256" % name)
    if not component["url"].startswith("https://"):
        raise SystemExit("%s is fetched over something other than https" % name)
    if not component["url"].endswith(component["archive"]) \
            and component["archive"] not in component["url"]:
        # An archive renamed on the way in (a tag-named GitHub tarball) is fine;
        # an archive that has nothing to do with its URL is not.
        if component["version"] not in component["url"]:
            raise SystemExit("%s: the archive and the url disagree" % name)

pinned = {c["name"]: c["version"] for c in lock["components"]}
if not pinned.get("gtk", "").startswith("4.16."):
    raise SystemExit("gtk is not pinned inside 4.16.x")
if not pinned.get("libadwaita", "").startswith("1.6."):
    raise SystemExit("libadwaita is not pinned inside 1.6.x")

# Section 4.1's private column, every row of it. A component dropped from the
# lock file is a library the package would take from the host without saying so.
required = {
    "glib", "gtk", "libadwaita", "pango", "cairo", "harfbuzz", "fribidi",
    "graphene", "gdk-pixbuf", "librsvg", "libwebp", "webp-pixbuf-loader",
    "libpng", "libjpeg-turbo", "libtiff", "libepoxy", "fontconfig", "freetype",
    "wayland", "wayland-protocols", "libffi", "pcre2", "appstream", "libxmlb",
    "libxml2", "adwaita-icon-theme", "dconf",
}
missing = required - set(names)
if missing:
    raise SystemExit("the lock file is missing: %s" % ", ".join(sorted(missing)))

glib = next(c for c in lock["components"] if c["name"] == "glib")
for flag in ("-Dselinux=disabled", "-Dlibmount=disabled"):
    if flag not in glib["options"]:
        raise SystemExit("glib is not built with %s" % flag)

gtk = next(c for c in lock["components"] if c["name"] == "gtk")
for flag in ("-Dx11-backend=true", "-Dwayland-backend=true",
             "-Dintrospection=disabled", "-Dmedia-gstreamer=disabled",
             "-Dprint-cups=disabled", "-Dcloudproviders=disabled",
             "-Dtracker=disabled", "-Dcolord=disabled", "-Dsysprof=disabled",
             "-Dvulkan=disabled"):
    if flag not in gtk["options"]:
        raise SystemExit("gtk is not built with %s" % flag)

hosts = set(lock["host_libraries"])
for needed in ("libc.so.6", "libgcc_s.so.1", "libGL.so.1", "libEGL.so.1",
               "libdbus-1.so.3", "libX11.so.6", "libxkbcommon.so.0",
               "libz.so.1", "libyaml-0.so.2", "libcurl.so.4"):
    if needed not in hosts:
        raise SystemExit("%s is not on the host list" % needed)
for forbidden in ("libgtk-4.so.1", "libglib-2.0.so.0", "libpango-1.0.so.0",
                  "libcairo.so.2", "libselinux.so.1", "libmount.so.1",
                  "libstdc++.so.6"):
    if forbidden in hosts:
        raise SystemExit("%s is on the host list and must not be" % forbidden)

print("  ok: %d components, every digest, every required flag" % len(names))
PY

echo "build_runtime_test: the meson flags, against upstream"
python3 "$RUNTIME_DIR/check_options.py" \
  --lock "$LOCK" \
  --sources "${FERMIX_RUNTIME_SOURCES:-${XDG_CACHE_HOME:-$HOME/.cache}/fermix-desktop-runtime/sources}" \
  || fail "a meson flag is not an option upstream declares"

echo "build_runtime_test: the container it declares"
[ -f "$DOCKERFILE" ] || fail "no runtime container at $DOCKERFILE"

grep -q '^FROM almalinux@sha256:' "$DOCKERFILE" \
  || fail "the base image is not pinned by digest"
grep -q 'sha256sum -c -' "$DOCKERFILE" || fail "patchelf is fetched without a digest check"
for pinned in RUST_VERSION MESON_VERSION NINJA_VERSION PATCHELF_VERSION; do
  grep -qE "ARG $pinned=[0-9]+\.[0-9]+\.[0-9]+" "$DOCKERFILE" \
    || fail "$pinned is not pinned to a version"
done
echo "  ok: the base image and every tool are pinned"

# The image must carry the development headers of the host half and none of the
# private half: a libgtk-4-devel here would be a way for the build to link the
# host's toolkit without anybody noticing.
# The four at the end are each here because a component's build stops dead
# without it and says so only after an hour of compiling. appstream's data/ uses
# itstool. libadwaita compiles its stylesheet with sassc or else downloads its
# own sassc subproject mid-build, which would be an unpinned dependency. xsltproc
# and the Docbook stylesheets are kept for the one component that would want them
# if patches/appstream-no-man-pages.patch were ever dropped; that patch says why
# they are not enough on their own.
for needed in mesa-libGL-devel mesa-libEGL-devel libX11-devel libxkbcommon-devel \
              dbus-devel zlib-devel libcurl-devel libyaml-devel jq patchelf \
              itstool libxslt docbook-style-xsl sassc; do
  grep -q "$needed" "$DOCKERFILE" || fail "the runtime container does not install $needed"
done
for forbidden in gtk4-devel glib2-devel pango-devel cairo-devel harfbuzz-devel \
                 gdk-pixbuf2-devel libadwaita-devel; do
  if grep -q "$forbidden" "$DOCKERFILE"; then
    fail "the runtime container installs $forbidden, which the runtime builds itself"
  fi
done
echo "  ok: the host headers are present and no private toolkit is"

echo "build_runtime_test: every refusal fired"

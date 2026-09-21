#!/usr/bin/env bash
#
# Exercise install_smoke.sh's refusals, its container's declarations and the
# assertions it exists to make, without running the smoke.
#
# The gate itself takes minutes per row and needs a privileged container; its
# argument handling takes milliseconds, and a gate that accepted a package path
# that does not exist, or an image whose package manager it does not know, would
# report a pass having installed nothing. Docker is never invoked here.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT="$ROOT_DIR/scripts/install_smoke.sh"
DOCKERFILE="$ROOT_DIR/packaging/docker/Dockerfile.smoke"
RELEASING="$ROOT_DIR/docs/RELEASING.md"

WORK="$(mktemp -d "${TMPDIR:-/tmp}/install-smoke-test.XXXXXX")"
trap 'rm -rf -- "$WORK"' EXIT

fail() {
  echo "install_smoke_test: $*" >&2
  exit 1
}

expect_refusal() {
  local what="$1"
  shift
  if "$@" >/dev/null 2>&1; then
    fail "$what was accepted"
  fi
  echo "  refused: $what"
}

echo "install_smoke_test: the script parses"
bash -n "$SCRIPT" || fail "the script does not parse"
echo "  ok: shell syntax"

echo "install_smoke_test: refusals"
printf 'not a package\n' > "$WORK/fermix-desktop.deb"
printf 'not a package\n' > "$WORK/fermix-desktop.rpm"
printf 'not a package\n' > "$WORK/fermix-desktop.tar.gz"

expect_refusal "no arguments at all" bash "$SCRIPT"
expect_refusal "two packages, which is the shape this gate no longer has" \
  bash "$SCRIPT" "$WORK/fermix-desktop.deb" "$WORK/fermix-desktop.rpm"
expect_refusal "a package that does not exist" bash "$SCRIPT" "$WORK/missing.deb"
expect_refusal "an argument nobody reads" \
  bash "$SCRIPT" --wayland-please "$WORK/fermix-desktop.deb"
expect_refusal "an image flag with no image" \
  bash "$SCRIPT" --image
expect_refusal "an image whose package manager is not known" \
  bash "$SCRIPT" --image slackware:15 "$WORK/fermix-desktop.deb"
expect_refusal "a deb against an rpm image" \
  bash "$SCRIPT" --image fedora:44 "$WORK/fermix-desktop.deb"
expect_refusal "an rpm against a deb image" \
  bash "$SCRIPT" --image ubuntu:22.04 "$WORK/fermix-desktop.rpm"
expect_refusal "a file that is neither a deb nor an rpm" \
  bash "$SCRIPT" "$WORK/fermix-desktop.tar.gz"

mkdir -p "$WORK/empty-bin"
expect_refusal "a host with no container runtime" \
  env PATH="$WORK/empty-bin" bash "$SCRIPT" "$WORK/fermix-desktop.deb"

echo "install_smoke_test: the container it declares"
[ -f "$DOCKERFILE" ] || fail "no smoke container at $DOCKERFILE"

# Each of these is a gate's dependency rather than a convenience, and a missing
# one shows up as a smoke that looks broken rather than as a named refusal:
# systemd because a user unit with no user manager is a file nobody reads,
# polkit because linger for another account goes through it, and the X tools
# because the evidence this gate produces is a picture of a window and the class
# that window carries.
for needed in systemd dbus polkitd xvfb xauth xdotool imagemagick; do
  grep -q "$needed" "$DOCKERFILE" ||
    fail "the smoke container does not install $needed for the deb family"
done
for needed in polkit xorg-x11-server-Xvfb xorg-x11-xauth xdotool ImageMagick; do
  grep -q "$needed" "$DOCKERFILE" ||
    fail "the smoke container does not install $needed for the rpm family"
done
echo "  ok: both families install every tool the gate uses"

# One Dockerfile, every row. A second Dockerfile per family is how the two
# halves of a matrix drift apart.
grep -q '^ARG BASE=' "$DOCKERFILE" ||
  fail "the smoke container is not parameterised by base image"
# shellcheck disable=SC2016  # ${BASE} is the literal Dockerfile text being looked for
grep -q '^FROM \${BASE}' "$DOCKERFILE" ||
  fail "the smoke container does not build FROM its BASE argument"
grep -q '^ARG WAYLAND=' "$DOCKERFILE" ||
  fail "the smoke container cannot be asked for a compositor"
grep -q 'weston' "$DOCKERFILE" ||
  fail "the smoke container installs no compositor for the Wayland rows"
grep -q 'epel-release' "$DOCKERFILE" ||
  fail "the rpm family has no EPEL, so AlmaLinux carries neither xdotool nor ImageMagick"
echo "  ok: one container, parameterised by base image and by compositor"

# The official images throw away /usr/share/doc on unpack, ubuntu through
# dpkg.cfg.d/excludes and fedora through tsflags=nodocs, while `dpkg -L` and
# `rpm -ql` go on listing what was thrown away. The container undoes it so that
# the row resembles a machine rather than an image -- the copyright file is the
# licence's own text and ought to be there. What the gate does NOT do is require
# the runtime manifest on disk afterwards, because that exclusion is a real
# configuration a real person can set: the manifest is checked in the package,
# where it is guaranteed, and the installed tree is asked for identity.json
# under the private prefix, where no exclusion reaches.
grep -q 'rm -f /etc/dpkg/dpkg.cfg.d/excludes' "$DOCKERFILE" ||
  fail "the deb images still throw away /usr/share/doc, so the runtime manifest cannot be checked"
grep -q 'tsflags=nodocs' "$DOCKERFILE" ||
  fail "the rpm images still throw away /usr/share/doc, so the runtime manifest cannot be checked"
echo "  ok: the images no longer strip the documentation the package ships"

# The same shape of problem, one image further on: AlmaLinux masks
# systemd-logind by symlinking the unit to /dev/null, and with no logind there
# is no linger, no lingering user manager, and nowhere to install a user unit.
# That row would fail for a reason belonging to the image, not to the package.
grep -q 'rm -f /etc/systemd/system/systemd-logind.service' "$DOCKERFILE" ||
  fail "the rpm images still mask systemd-logind, so linger cannot be granted"
echo "  ok: logind is unmasked, so linger and the user manager are real"

# Weston's headless backend composites nothing under its default renderer, so
# the capture protocol hands the screenshooter a zero-sized frame and it dies on
# an assertion; and the compositor refuses that protocol to every client unless
# it was started with --debug. Either flag missing turns the Wayland rows into a
# failure that reads as the application never having drawn.
grep -q -- '--renderer=pixman' "$SCRIPT" ||
  fail "weston runs headless with no renderer, so any capture of it is zero-sized"
grep -q -- '--debug' "$SCRIPT" ||
  fail "weston runs without --debug, so the screenshooter is refused the capture protocol"
echo "  ok: the compositor both renders and lets the screenshooter capture"

grep -q 'useradd --create-home' "$DOCKERFILE" ||
  fail "the smoke container has no ordinary account to install the service as"
grep -q 'CMD \["/sbin/init"\]' "$DOCKERFILE" ||
  fail "the smoke container does not run systemd as process one"
grep -q 'STOPSIGNAL SIGRTMIN+3' "$DOCKERFILE" ||
  fail "the smoke container cannot be stopped cleanly, so a run leaks a container"
echo "  ok: systemd is process one, and one ordinary account owns the service"

echo "install_smoke_test: what the gate asserts"
# The facts the smoke exists to prove. A script that stopped asserting one of
# them would still exit zero, so the assertions themselves are checked here.
for assertion in \
  '"ok":true' \
  '"alignment":"aligned"' \
  '"enabled":true' \
  '"active":true' \
  '/usr/bin/fermix-desktop --version' \
  'xdotool search --class' \
  'test -L /usr/bin/fermix-desktop' \
  '/usr/lib/fermix-desktop/bin/fermix-desktop' \
  '/usr/lib/fermix-desktop/lib/gio/modules/libdconfsettings.so' \
  'usr/share/doc/fermix-desktop/runtime-manifest.json' \
  'dpkg-deb -c' \
  'rpm -qlp' \
  '/usr/lib/fermix-desktop/share/fermix-desktop-runtime/identity.json' \
  '"cache_key"' \
  '"engine_tree_sha256"' \
  '/var/lib/fermix/runtimes' \
  'GDK_BACKEND=wayland' \
  "su - test -c 'command -v secret-tool'" \
  'systemctl --user show-environment' \
  '/usr/lib/fermix-desktop/libexec/$helper' \
  'head -c 4' \
  'runtimez' \
  'json.load(sys.stdin)' \
  'parses as JSON from byte 0' \
  'gio-launch-desktop dconf-service' \
  'weston-screenshooter'; do
  grep -qF -- "$assertion" "$SCRIPT" ||
    fail "the smoke no longer asserts $assertion"
done
echo "  ok: the install, the private prefix, the loader store, the alignment,"
echo "      the version, the window class and the Wayland draw"

# `secret-tool` in BOTH environments, and the same file in each. The engine
# shells out to it to save a channel bot key, and the application and the
# service run under different PATHs -- the desktop session's and the user
# manager's, which are genuinely different lists and were measured rather than
# assumed on all seven images. Resolving in one environment says nothing about
# the other, and resolving to two different binaries would have the app and the
# service saving through different helpers. The comparison is by device and
# inode so a symlink to the same file is not read as a difference.
for assertion in \
  'systemctl --user show-environment' \
  "stat -L -c '%d:%i'" \
  'DIFFERENT files'; do
  grep -qF -- "$assertion" "$SCRIPT" ||
    fail "the smoke no longer proves secret-tool is one file in both environments: $assertion"
done
echo "  ok: secret-tool resolves for both the session and the unit, to one file"

# THE UPGRADE ROW. Every other row installs onto a clean machine, and the owner
# did not have one: on 2026-09-20 they installed over a running engine and got a
# new application talking to an OLD engine -- not for the minutes before a
# restart, but permanently, because the extracted payload directory is keyed on
# product version alone, so the new payload is never unpacked and a restart
# produces a new process running the same old code. Nothing in this file could
# see it, because nothing in this file had ever upgraded anything.
echo "install_smoke_test: the upgrade row"
grep -q -- '--upgrade-from' "$SCRIPT" ||
  fail "the smoke cannot install an older package first, so it never upgrades anything"
for assertion in \
  'secret.migrate_to_keyring' \
  'engine_build_id' \
  'pending_restart'; do
  grep -qF -- "$assertion" "$SCRIPT" ||
    fail "the upgrade row does not assert $assertion"
done

# The EXTRACTED payload, not the packaged binary. The wrapper on disk is new
# whether or not it ever unpacked anything, so grepping it proves the package
# carries the change and says nothing about what the daemon is running. What
# separates them is a module present only in the new engine, found in the
# extraction -- with an invented module name that must be ABSENT, or "present"
# could just mean the search matches anything.
grep -q 'burrito' "$SCRIPT" ||
  fail "the upgrade row never looks at the extracted payload, only at the installed files"
grep -q 'NoSuchModule' "$SCRIPT" ||
  fail "the extracted-payload search has no negative control"

# The extraction moved. The launcher now names the directory after the payload
# digest under ${XDG_CACHE_HOME:-$HOME/.cache}/fermix/runtime, so a probe that
# only knows the old default would report an old payload when it is merely
# somewhere else -- a gate that fails when the fix succeeds. Both roots are
# searched, because the old default still applies wherever the variable is unset.
grep -q 'fermix/runtime' "$SCRIPT" ||
  fail "the extraction probe does not search the launcher's identity-keyed cache directory"

# The launcher changes the unit's command line, and nothing running as root can
# reload another account's user manager. Without a reload in the product's own
# restart path, a restart faithfully relaunches the superseded command.
grep -q 'need_daemon_reload' "$SCRIPT" ||
  fail "the upgrade row does not check that the restart left no pending daemon-reload"

# The running engine is the one that ANSWERS, not the one on disk. Asserting
# that the package's files changed proves nothing here: they did change, and the
# old engine went on serving regardless.
grep -q 'ENGINE THAT ANSWERS' "$SCRIPT" ||
  fail "the upgrade row does not say why it reads the running engine rather than the installed files"
echo "  ok: an older package, a running service, an upgrade, a restart, and what answers after it"

# Is this the package the run was TOLD to test? `--expect-tree` cannot answer
# that: two builds minutes apart carry the same engine tree and differ only in
# app code, which is exactly the pair that reached this gate on 2026-09-20. The
# package's own digest is the only thing that separates them, so the row can be
# handed one and refuse every other file.
echo "install_smoke_test: the package is the one it was told to test"
grep -q -- '--expect-package' "$SCRIPT" ||
  fail "the row cannot be told which package digest it is meant to be testing"
grep -q 'sha256sum' "$SCRIPT" ||
  fail "the row never hashes the package it was handed"
echo "  ok: the package can be pinned by digest, not only by engine tree"

# Does the INSTALLED BINARY carry the change this build exists to ship?
#
# A package built from a tree taken minutes before a fix looks identical from
# the outside: same version, same size to within a rounding, and it passes every
# other row here. One was handed to this gate on 2026-09-20 and did exactly
# that, while missing the bounded retry that acceptance rows 18 and 19 promise a
# reviewer -- so those rows would have failed for a reason that has nothing to
# do with the rows.
#
# The probe needs its own positive control, because the TOOL can be missing:
# `strings` is not present in debian:13, and a probe that uses it there reports
# a clean zero for every string. Absence of the change and absence of the tool
# would otherwise be the same answer, which is the worst shape a check can have.
# STDOUT PURITY. A --json verb whose stdout carries anything but JSON breaks
# every consumer of it, and the failure is invisible to a gate that only checks
# exit status or greps for a field. It happened on 2026-09-20: the engine
# wrapper printed two informational lines ahead of the JSON whenever an install
# directory variable was set, and every gate on every side passed it.
echo "install_smoke_test: what the engine prints on stdout"
for assertion in \
  'check_json_commands_emit_only_json' \
  'from its first byte' \
  'a banner in front of JSON'; do
  grep -qF -- "$assertion" "$SCRIPT" ||
    fail "the smoke does not check that a --json verb prints JSON and nothing else: $assertion"
done
echo "  ok: --json stdout is parsed from byte 0, and a banner before it would be refused"

# A search that correctly finds nothing must not end the run. That is how
# check_loader_store broke every row on 2026-09-20: exit 1, no message, three
# runs spent looking for a failure that had no output. A second, latent instance
# sat in the same function, where the refusal that says "names no loader" could
# never have printed.
#
# The rule, measured on 2026-09-20 rather than reasoned about, in three clauses:
#
#   1. only an ASSIGNMENT from a substitution can kill the script. The same
#      substitution used as an argument -- inside `[ ]`, as a word -- takes the
#      enclosing command's status instead;
#   2. under pipefail a search ANYWHERE in the pipeline propagates. No ending
#      saves it: `x="$(grep nope f | wc -l)"` dies, ending in `wc -l` and all.
#      That is the clause I had wrong, and had it stood the next person would
#      have written exactly that believing the ending protected them;
#   3. a nested `sh -c "A | B"` insulates, because the inner shell has no
#      pipefail and only B's status escapes. That -- not the ending -- is why
#      the probes in this gate are safe.
#
# ONE RULE, and it is a claim about POSITION that a text check can verify: an
# assignment whose substitution searches outside a quoted `sh -c` must end that
# substitution with `|| true` immediately before its closing paren.
#
# Four exemptions were tried before this, and each was "the line contains X"
# standing in for "X sits here": the pipeline's ending, the presence of `sh -c`,
# `sh -c` with its quoted command stripped, and finally any `||` anywhere on the
# statement — which allowed `x="$(… | grep nope)"; [ -n "$x" ] || echo`, where
# the fallback attaches to the test and the assignment has already died. Each
# was blind to the case it was written for. A fallback that must sit immediately
# before the closing paren cannot be satisfied by an unrelated one elsewhere.
#
# Both quote styles are stripped, because `sh -c '…'` insulates exactly as
# `sh -c "…"` does. Statements are joined across backslash continuations first.
unsafe="$(awk '
  { line = $0 }
  joined != "" { line = joined " " line; joined = "" }
  line ~ /\\$/ { sub(/\\$/, "", line); joined = line; next }
  {
    stripped = line
    while (match(stripped, /sh -c[ ]*"[^"]*"/) || match(stripped, /sh -c[ ]*'"'"'[^'"'"']*'"'"'/)) {
      stripped = substr(stripped, 1, RSTART - 1) substr(stripped, RSTART + RLENGTH)
    }
    if (line ~ /=("|)\$\(/ && stripped ~ /grep|find/ && line !~ /\|\| true\)/) {
      print NR ": " line
    }
  }
' "$SCRIPT")"
[ -z "$unsafe" ] ||
  fail "a searching substitution does not end in '|| true)', so an empty result can end the run silently: $unsafe"
echo "  ok: no assignment can be killed by a search that correctly finds nothing"

echo "install_smoke_test: the fix is in the binary, not only in the tree"
for assertion in \
  'FIX_STRING' \
  'FIX_CONTROL' \
  'grep -c'; do
  grep -qF -- "$assertion" "$SCRIPT" ||
    fail "the smoke cannot say whether the installed binary carries a named change: $assertion"
done
grep -q 'the probe itself is broken' "$SCRIPT" ||
  fail "the binary probe has no positive control, so a missing tool would read as a missing fix"
echo "  ok: the binary is probed for a named string, with a control that fails loudly"

# The package contains the engine, so nothing here installs a second package and
# nothing here compares two versions. A smoke that still did would be proving a
# relation this product no longer has.
for gone in 'engine.deb' 'exact-version' 'fermix (= '; do
  if grep -qF -- "$gone" "$SCRIPT"; then
    fail "the smoke still talks about two packages: $gone"
  fi
done
echo "  ok: one package, and no relation between two of them to prove"

echo "install_smoke_test: every wait is bounded"
# A gate that waits forever does not fail, it times out in a job whose error
# names the job. Every loop here counts, and every cap has a sentence.
for cap in SYSTEMD_ATTEMPTS USER_MANAGER_ATTEMPTS WINDOW_ATTEMPTS WESTON_ATTEMPTS; do
  grep -qE "^$cap=[0-9]+$" "$SCRIPT" || fail "$cap is not a declared bound"
done
if grep -qE 'while true|until .*; do$' "$SCRIPT"; then
  fail "the smoke carries an unbounded loop"
fi
echo "  ok: four declared caps, and no unbounded loop"

echo "install_smoke_test: the matrix the documentation names is runnable"
# Every image the release documentation names has to be one this script knows a
# package manager for, or the release rail asks for a row the gate refuses.
[ -f "$RELEASING" ] || fail "no release documentation at $RELEASING"
# shellcheck disable=SC2016  # the backticks are markdown, matched literally
images="$(grep -oE '`(ubuntu|debian|fedora|almalinux):[0-9.]+`' "$RELEASING" |
  tr -d '`' | LC_ALL=C sort -u || true)"
[ -n "$images" ] || fail "the release documentation names no smoke images"
for image in $images; do
  case "${image%%:*}" in
    ubuntu | debian | fedora | almalinux) ;;
    *) fail "the documentation names $image and install_smoke.sh has no family for it" ;;
  esac
done
# The two floors of the amendment stay floors: the oldest deb target and the
# oldest rpm target are the rows that prove the glibc floor and the host GTK
# this package does not use.
for floor in ubuntu:22.04 almalinux:9; do
  case "$images" in
    *"$floor"*) ;;
    *) fail "the documentation no longer names the floor $floor" ;;
  esac
done
echo "  ok: $(printf '%s' "$images" | tr '\n' ' ')"

echo "install_smoke_test: every refusal fired"

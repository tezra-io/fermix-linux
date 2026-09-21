#!/usr/bin/env bash
#
# Install the one package on a machine that has a systemd, and see the window
# open against a real daemon (M38 section 13.1, amendment section 8.4).
#
# Everything before this gate reads files. This one installs what was built, on
# a host whose package manager resolves the declared relations out of the
# distribution's own archive, under an ordinary account whose own
# `systemd --user` runs the engine.
#
# What it proves:
#
#   1. the family's package manager installs the file, which means every
#      relation the package declares is satisfiable from that distribution's own
#      archive under the name the package writes down;
#   2. the package installs exactly the files it claims, at the paths the window
#      and the engine read them from, the private runtime prefix included, and
#      the postinstall materialised the loader store;
#   3. `fermix service install --json` brings the engine up under this account's
#      own user manager, and `fermix service status --json` then reports the
#      installed and running builds as aligned;
#   4. `fermix-desktop --version` answers from the packaged binary and names the
#      build id the installed manifest names;
#   5. the window opens against that real daemon, with no fixture home and no
#      fake command line, and the window the display server sees carries the
#      application identity as its class, which is the one place that string is
#      checked against a running window rather than against a file;
#   6. with `--wayland`, the window also draws through the private
#      `libwayland-client` on a headless Weston, which is the component this
#      design replaces at a version the host does not have and the component
#      whose failure mode is a window that never appears.
#
# What it deliberately does not prove: anything that needs a desktop. A portal
# backend, a notification daemon, a shell that owns a tray watcher, an accent
# colour, a person. Those are the hand-verified gates of M38 section 13.2, and
# docs/ACCEPTANCE_RUNBOOK.md carries them.
#
# One thing is arranged rather than exercised, and it is worth saying: linger is
# granted to the account by root before the engine's own install runs. The
# engine skips its `loginctl enable-linger` when linger is already on, so this
# gate does not exercise the polkit hop. The engine's own suite covers that path
# in both of its shapes; what is being proven here is packaging.
#
# Usage: install_smoke.sh [--image <image>] [--wayland] [--expect-package <sha256>] [--keep] <package>
#   <package>   one deb or one rpm, the whole product
#   --image     the base image of the row to run. Defaults to ubuntu:22.04, the
#               oldest deb target
#   --wayland   draw the window a second time on a headless Weston. Only the
#               newest image of each family carries a Weston that runs headless
#               without a backport, so only those rows pass this
#   --expect-package  the sha256 this package must hash to. Two builds minutes
#               apart share an engine tree and differ only in app code, so the
#               digest is the only thing that tells them apart
#   --keep      leave the container running afterwards, to look inside it
#
# Evidence lands in packaging/out/smoke/<image>/.
set -euo pipefail

USAGE="usage: install_smoke.sh [--image <image>] [--wayland] [--upgrade-from <older package>] [--expect-package <sha256>] [--keep] <package>"

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DOCKERFILE="$ROOT_DIR/packaging/docker/Dockerfile.smoke"
APP_ID="io.tezra.Fermix"
HOME_PATH="/home/test/fermix home"

# Every wait in this script is bounded, and every cap is a refusal naming what
# never happened rather than a loop that runs until the job times out.
SYSTEMD_ATTEMPTS=60
USER_MANAGER_ATTEMPTS=30
WINDOW_ATTEMPTS=40
WESTON_ATTEMPTS=30

PACKAGE=""
IMAGE="ubuntu:22.04"
# The package digest this run was told to test. Empty means unpinned.
EXPECT_PACKAGE=""
# An older package to install FIRST, so this row upgrades over a running engine
# instead of installing onto a clean machine. Empty means the ordinary row.
UPGRADE_FROM=""
WAYLAND=0
KEEP=0

CONTAINER="fermix-desktop-smoke-$$"
TAG=""
EVIDENCE=""
FAMILY=""
WORK=""

fail() {
  echo "install_smoke: $*" >&2
  exit 1
}

step() {
  echo
  echo "install_smoke: == $*"
}

# Every path out of this script goes through here: the container is removed and
# the temporary directory with it, on success, on refusal and on interrupt.
cleanup() {
  if [ "$KEEP" = "1" ]; then
    echo "install_smoke: $CONTAINER is still running, as asked"
  else
    docker rm -f "$CONTAINER" >/dev/null 2>&1 || true
  fi
  [ -z "$WORK" ] || rm -rf -- "$WORK"
}

parse_arguments() {
  local positional=()
  while [ "$#" -gt 0 ]; do
    case "$1" in
      --image)
        [ "$#" -ge 2 ] || fail "--image needs an image ($USAGE)"
        IMAGE="$2"
        shift 2
        ;;
      --wayland)
        WAYLAND=1
        shift
        ;;
      --upgrade-from)
        [ "$#" -ge 2 ] || fail "--upgrade-from needs an older package ($USAGE)"
        UPGRADE_FROM="$2"
        shift 2
        ;;
      --expect-package)
        [ "$#" -ge 2 ] || fail "--expect-package needs a sha256 ($USAGE)"
        EXPECT_PACKAGE="$2"
        shift 2
        ;;
      --keep)
        KEEP=1
        shift
        ;;
      --*) fail "unknown argument '$1' ($USAGE)" ;;
      *)
        positional+=("$1")
        shift
        ;;
    esac
  done
  [ "${#positional[@]}" -eq 1 ] || fail "exactly one package is required ($USAGE)"
  PACKAGE="${positional[0]}"
  check_package_is_the_expected_one
}

# Is this the package the run was told to test?
#
# `--expect-tree` cannot answer that. Two builds taken minutes apart carry the
# same engine tree and differ only in app code, and that is exactly the pair
# that reached this gate on 2026-09-20: identical engine, identical contract,
# identical packaging, and an application compiled before a fix. Every identity
# check passed and none of them could see it. The package's own digest is the
# one thing that separates them, so a run can be handed a digest and refuse any
# other file before it starts a container.
#
# Optional, because the row must stay usable on a package nobody has quoted a
# digest for. Given one, it is a refusal rather than a warning.
check_package_is_the_expected_one() {
  [ -n "$EXPECT_PACKAGE" ] || return 0
  [ -f "$PACKAGE" ] || fail "$PACKAGE is not a file, so its digest cannot be checked"
  local actual
  actual="$(sha256sum "$PACKAGE" | cut -d' ' -f1)"
  [ "$actual" = "$EXPECT_PACKAGE" ] ||
    fail "this is not the package this run was told to test: $PACKAGE hashes to $actual, expected $EXPECT_PACKAGE"
  echo "install_smoke: the package is $actual, which is the one this run was told to test"
}

# The family decides the package manager and the install command. An allowlist
# rather than a guess: an image nobody has run this against is a refusal, not a
# run that installs nothing and reports a pass.
family_of() {
  case "${1%%:*}" in
    ubuntu | debian) printf '%s\n' "deb" ;;
    fedora | almalinux | rockylinux | centos) printf '%s\n' "rpm" ;;
    *)
      echo "install_smoke: no package manager is known for the image '$1'" >&2
      return 1
      ;;
  esac
}

guard() {
  command -v docker >/dev/null 2>&1 || fail "docker is not installed"
  [ -f "$DOCKERFILE" ] || fail "no smoke container at $DOCKERFILE"
  [ -f "$PACKAGE" ] || fail "no package at $PACKAGE"

  case "$PACKAGE" in
    *.deb) [ "$FAMILY" = "deb" ] || fail "$PACKAGE is a deb and $IMAGE is an rpm image" ;;
    *.rpm) [ "$FAMILY" = "rpm" ] || fail "$PACKAGE is an rpm and $IMAGE is a deb image" ;;
    *) fail "$PACKAGE is neither a deb nor an rpm" ;;
  esac
}

inside() {
  docker exec "$CONTAINER" "$@"
}

# As the ordinary account, with the runtime directory its user manager owns.
#
# Note what this does not reproduce. A machine has several PATHs and this helper
# uses none of the interesting ones: `docker exec` inherits the image's own ENV,
# which is a fourth thing that matches no consumer of the product. The two that
# matter are the desktop session's and the one the engine's unit is given, and
# they differ per family. Measured on every image of the matrix, with the test
# account lingering and its user manager up:
#
#   ubuntu:22.04  login  /usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin + games + snap
#                 unit   identical to the login shell
#   ubuntu:24.04  login and unit identical, the same list as 22.04
#   ubuntu:26.04  login and unit identical, the same list as 22.04
#   debian:12     login  /usr/local/bin:/usr/bin:/bin:/usr/local/games:/usr/games
#                 unit   /usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
#   debian:13     login  /usr/local/bin:/usr/bin:/bin:/usr/local/games:/usr/games
#                 unit   /usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin
#   almalinux:9   login  ~/.local/bin:~/bin:/usr/local/bin:/usr/bin:/usr/local/sbin:/usr/sbin
#                 unit   /usr/local/bin:/usr/local/sbin:/usr/bin:/usr/sbin
#   fedora:44     login  ~/.local/bin:~/bin:/usr/local/bin:/usr/bin
#                 unit   /usr/local/bin:/usr/bin
#
# The direction reverses across families: Ubuntu hands both the same list,
# Debian gives the unit sbin directories the session does not have, and
# AlmaLinux and Fedora give the session two home directories the unit does not.
# **Neither is reliably the subset**, so no single environment, finding a
# program, proves the other can. Anything asking whether a program can be found
# must ask in both and say which one failed: a session failure means the
# application cannot reach it, a unit failure means `fermix.service` cannot, and
# those are different bugs. AlmaLinux also orders the same set of system
# directories differently in the two, so a program installed in more than one of
# them can resolve to different files. This helper is for running the product,
# not for resolving programs.
as_test() {
  docker exec \
    -u test \
    -e "XDG_RUNTIME_DIR=/run/user/$(inside id -u test | tr -d '\r')" \
    -e HOME=/home/test \
    "$CONTAINER" "$@"
}

start_machine() {
  step "the machine: $IMAGE"
  docker build \
    -f "$DOCKERFILE" \
    --build-arg "BASE=$IMAGE" \
    --build-arg "WAYLAND=$WAYLAND" \
    -t "$TAG" \
    "$ROOT_DIR/packaging/docker" ||
    fail "the smoke container for $IMAGE did not build"

  # systemd as process one needs the cgroup filesystem and a writable /run.
  # This is a throwaway container, and it is torn down on every exit path.
  docker run -d --name "$CONTAINER" \
    --privileged \
    --cgroupns=host \
    --tmpfs /run --tmpfs /run/lock --tmpfs /tmp \
    -v /sys/fs/cgroup:/sys/fs/cgroup:rw \
    "$TAG" >/dev/null ||
    fail "the smoke container did not start"

  local attempt
  echo -n "  waiting for systemd"
  for attempt in $(seq 1 "$SYSTEMD_ATTEMPTS"); do
    case "$(inside systemctl is-system-running 2>/dev/null || true)" in
      running | degraded)
        echo
        echo "  systemd is up after ${attempt}s"
        return 0
        ;;
    esac
    echo -n "."
    sleep 1
  done
  echo
  fail "systemd never came up in the container within ${SYSTEMD_ATTEMPTS}s"
}

# One command, the one an operator runs. The package is handed to the package
# manager as a local file so that its declared relations are resolved out of the
# distribution's own archive around it, which is the whole point: a relation the
# archive does not carry under that name fails here.
# The upgrade row: an older engine, running, and this package installed over it.
#
# Every other row in this file installs onto a clean machine, and the owner did
# not have one. On 2026-09-20 they installed over a running engine and their new
# application talked to the OLD engine -- and not merely until the next restart.
# The extracted payload directory is named for the product version alone, so at
# the same version the new payload is never unpacked: restarting produced a
# fresh process running the same old code, while `service status` reported
# "aligned" throughout, because both halves of that comparison came from the old
# build.
#
# Which is why this row reads THE ENGINE THAT ANSWERS rather than the files on
# disk. The files did change. Asserting that they changed is precisely the check
# that passed all evening while the owner ran an engine from hours earlier.

# What answers after the upgrade and the restart?
#
# The product's own way of restarting is what the application drives, so this
# restarts the user unit rather than rebooting the container: the question is
# whether a restart is ENOUGH, which is exactly what failed for the owner.
check_the_upgrade_replaced_the_engine() {
  step "after the upgrade: what answers now"
  local uid expected_build
  uid="$(inside id -u test | tr -d '\r')"
  expected_build="$(inside sed -n 's/.*"engine_build_id"[: ]*"\([^"]*\)".*/\1/p' \
    /usr/share/fermix-desktop/build.json | tr -d '\r')"
  [ -n "$expected_build" ] ||
    fail "the installed build.json names no engine_build_id, so there is nothing to compare against"

  # Before the restart the engine is still the old one, and the product is
  # supposed to SAY so. Reporting "aligned" here is the failure that left the
  # owner with no prompt to act on.
  local before
  before="$(as_test fermix service status --json | tr ',' '\n' |
    sed -n 's/.*"alignment":"\{0,1\}\([a-z_]*\)"\{0,1\}.*/\1/p' | head -1 | tr -d '\r')"
  printf '%s\n' "$before" > "$EVIDENCE/alignment_before_restart.txt"
  echo "  before the restart the product reports alignment: $before"
  [ "$before" = "pending_restart" ] ||
    fail "after an upgrade and before a restart the product reports '$before', not pending_restart: nothing will tell the owner to act"

  # THE PRODUCT'S OWN RESTART, not `systemctl --user restart`. That distinction
  # is the whole point of this step: a package upgrade rewrites the unit, and
  # nothing running as root can reload another account's user manager, so
  # `fermix restart` reloads it first. Restarting the unit directly reproduces
  # the owner's failure rather than testing the fix -- measured on 2026-09-21,
  # where a direct restart left the daemon unreachable and systemd warned that
  # the unit on disk had changed.
  local restarted
  restarted="$(as_test fermix restart --json 2>&1)" ||
    fail "the product's own restart refused: $restarted"
  printf '%s\n' "$restarted" > "$EVIDENCE/restart.json"
  await_engine_after_restart

  local running
  # The RUNNING half specifically. `installed` in this same response comes from
  # the identity compiled into the CLI binary that answered, and that CLI has an
  # extraction of its own -- so on a stale machine both halves can be the old
  # build for two different reasons.
  running="$(as_test fermix service status --json |
    sed 's/.*"running"://' | tr ',' '\n' |
    sed -n 's/.*"build_id": *"\([^"]*\)".*/\1/p' | head -1 | tr -d '\r')"
  printf '%s\n' "$running" > "$EVIDENCE/running_build_after_restart.txt"
  [ "$running" = "$expected_build" ] ||
    fail "after the upgrade and a restart the ENGINE THAT ANSWERS is $running and the package carries $expected_build: the new payload was never unpacked"
  echo "  the running engine is $running, which is the one this package carries"

  check_the_extracted_payload_is_the_new_engine
  check_the_daemon_publishes_the_new_verb
  check_no_reload_is_still_pending
  check_the_stale_payload_was_pruned
  check_a_cli_run_leaves_the_service_payload_alone
}

# The unkeyed extraction from before the keying change must be cleaned up, and
# the daemon says so on start. Asserted from the journal rather than by looking
# for an absence: a directory that was never there and a directory that was
# removed look identical on disk, and only one of them is the behaviour claimed.
check_the_stale_payload_was_pruned() {
  local line
  line="$(as_test journalctl --user -u fermix --no-pager -o cat |
    grep -m1 'pruned runtime payloads' || true)"
  printf '%s\n' "$line" > "$EVIDENCE/prune.txt"
  [ -n "$line" ] ||
    fail "the daemon logged no prune of the runtime payloads, so whether a stale unkeyed extraction would be cleaned up is unknown"
  case "$line" in
    *"removed 0"*)
      echo "  the daemon pruned nothing this run (no stale payload was present): $line"
      ;;
    *) echo "  $line" ;;
  esac
}

# A CLI run during an upgrade must not disturb the RUNNING daemon's payload.
#
# Burrito's wrapper cleans older siblings on every launch, so before the two
# roots were separated an ordinary `fermix --version` typed by the owner would
# have deleted the running daemon's extraction, and the daemon would have died
# later at a lazy module load. Two roots is what prevents it.
#
# BOTH halves are asserted, because either alone is vacuous: if the service copy
# merely survives, that is equally consistent with the cleaner never having run
# at all. The CLI copy disappearing is what proves it ran.
check_a_cli_run_leaves_the_service_payload_alone() {
  local older="fermix_linux_package_erts-16.3_0.10.4"
  # Suffixes, not '$HOME/...' strings. Python's expanduser expands `~` and NOT
  # `$HOME`, so passing the shell spelling plants a literal directory named
  # '$HOME' in the working directory while the shell-side checks look at the
  # real roots — both arms then report "nothing here" and the check fails
  # claiming the service copy was deleted. Measured on 2026-09-21; the product
  # was correct and the check was wrong.
  local cli_suffix=".local/share/.burrito"
  local service_suffix=".cache/fermix/runtime/.burrito"
  local cli_root='$HOME'"/$cli_suffix"
  local service_root='$HOME'"/$service_suffix"

  # The planted directories must carry METADATA THE CLEANER CAN READ, or it
  # ignores them and this check passes for the wrong reason: do_clean_old_versions
  # loads each sibling's `_metadata.json` and skips any it cannot parse.
  #
  # The script is COPIED IN and run by path, never fed on stdin. `as_test` is
  # `docker exec` without `-i`, so a heredoc reaches nothing: python3 read an
  # empty program, did nothing, exited 0, and this check then reported that the
  # product had deleted a directory that had never been created. Measured on
  # 2026-09-21, after three separate runs had established by hand that the
  # product behaves correctly.
  cat > "$WORK/plant.py" <<'PLANT'
import json, os, sys, glob
home = os.environ["HOME"]
cli_root = os.path.join(home, sys.argv[1])
service_root = os.path.join(home, sys.argv[2])
older = sys.argv[3]
source = None
for root in (cli_root, service_root):
    for path in glob.glob(os.path.join(root, "*", "_metadata.json")):
        source = path
        break
    if source:
        break
if source is None:
    raise SystemExit("no existing extraction to copy metadata from")
meta = json.load(open(source))
meta["app_version"] = "0.10.4"
for root in (cli_root, service_root):
    target = os.path.join(root, older)
    os.makedirs(target, exist_ok=True)
    json.dump(meta, open(os.path.join(target, "_metadata.json"), "w"))
print("planted")
PLANT
  docker cp "$WORK/plant.py" "$CONTAINER:/usr/local/lib/fermix-plant.py" >/dev/null ||
    fail "the planting script could not be copied into the container"

  # And the planting is CONFIRMED before the action under test, because a plant
  # that did nothing makes every arm below meaningless.
  as_test python3 /usr/local/lib/fermix-plant.py "$cli_suffix" "$service_suffix" "$older" > /dev/null ||
    fail "the older sibling extractions could not be planted, so this check would prove nothing"
  local planted_cli planted_service
  planted_cli="$(as_test sh -c "test -d \"$cli_root/$older\" && echo yes || echo no" | tr -d '\r')"
  planted_service="$(as_test sh -c "test -d \"$service_root/$older\" && echo yes || echo no" | tr -d '\r')"
  [ "$planted_cli" = "yes" ] && [ "$planted_service" = "yes" ] ||
    fail "the plant did not create both sibling extractions (cli=$planted_cli service=$planted_service), so this check would prove nothing"

  as_test fermix --version > /dev/null 2>&1 || true

  local cli_left service_left
  cli_left="$(as_test sh -c "test -d \"$cli_root/$older\" && echo yes || echo no" | tr -d '\r')"
  service_left="$(as_test sh -c "test -d \"$service_root/$older\" && echo yes || echo no" | tr -d '\r')"

  [ "$service_left" = "yes" ] ||
    fail "a CLI run deleted an extraction under the SERVICE root: an upgrade would take the running daemon's payload with it"
  [ "$cli_left" = "no" ] ||
    fail "the CLI run removed nothing under its own root either, so this check cannot tell protection from a cleaner that never ran"
  echo "  a CLI run pruned its own root and left the service root untouched"
}

# The unit's command line changed with the upgrade, and nothing running as root
# can reload another account's user manager -- so unless the product reloads it
# in its own restart path, the restart faithfully relaunches the superseded
# command and everything above this line can pass while the owner runs the old
# one. A pending reload after the product's own restart is that failure.
check_no_reload_is_still_pending() {
  local pending
  pending="$(as_test fermix service status --json |
    sed 's/.*"unit"://' | tr ',' '\n' |
    sed -n 's/.*"need_daemon_reload": *\([a-z]*\).*/\1/p' | head -1 | tr -d '\r')"
  [ -n "$pending" ] ||
    fail "service status reports no need_daemon_reload, so whether the unit was reloaded cannot be established"
  [ "$pending" = "false" ] ||
    fail "after the product's own restart the unit still needs a daemon-reload, so the running command line is the superseded one"
  echo "  no daemon-reload is pending: the unit that restarted is the installed one"
}

# What the DAEMON publishes, which is the only statement about what the
# application can actually call. A module in the extraction is an input; the
# published method list is the contents. Both are checked because they fail
# differently: an extraction can be new while a daemon that started before it
# goes on serving.
check_the_daemon_publishes_the_new_verb() {
  install_a_wire_client
  local hello
  hello="$(as_test python3 /usr/local/lib/fermix-wire.py "$HOME_PATH/daemon.sock" hello '{}')" ||
    fail "the daemon did not answer hello after the restart"
  printf '%s\n' "$hello" > "$EVIDENCE/hello_after_restart.json"

  case "$hello" in
    *'"secret.no_such_method"'*)
      fail "hello lists an invented method, so this check would accept anything"
      ;;
  esac
  case "$hello" in
    *'"secret.migrate_to_keyring"'*)
      echo "  the daemon publishes secret.migrate_to_keyring"
      ;;
    *)
      fail "after the upgrade and a restart the daemon does not publish secret.migrate_to_keyring: it is still the older engine"
      ;;
  esac
}

# The management socket speaks 4-byte length-prefixed JSON, so it needs a client
# rather than a command. Only the upgrade row installs one, and only python3,
# because the ordinary rows must keep running on the image as it ships.
install_a_wire_client() {
  if [ "$FAMILY" = "deb" ]; then
    inside apt-get install -y -qq python3 >/dev/null 2>&1 ||
      fail "python3 could not be installed, and the management socket needs a client"
  else
    inside dnf install -y python3 >/dev/null 2>&1 ||
      fail "python3 could not be installed, and the management socket needs a client"
  fi
  cat > "$WORK/wire.py" <<'PY_CLIENT'
import json, socket, struct, sys

sock_path, method, params = sys.argv[1], sys.argv[2], sys.argv[3]
body = json.dumps({"request_id": "upgrade-row",
                   "protocol_version": 2,
                   "method": method,
                   "params": json.loads(params)}).encode()

s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.settimeout(120)
try:
    s.connect(sock_path)
    s.sendall(struct.pack(">I", len(body)) + body)
    head = s.recv(4)
    if len(head) < 4:
        print('{"error":{"code":"no_response"}}')
        sys.exit(0)
    remaining = struct.unpack(">I", head)[0]
    buf = b""
    while len(buf) < remaining:
        chunk = s.recv(remaining - len(buf))
        if not chunk:
            break
        buf += chunk
    print(buf.decode())
finally:
    s.close()
PY_CLIENT
  # NOT /tmp. systemd mounts a tmpfs over /tmp inside the container, and
  # `docker cp` writes into the layer BENEATH that mount — so the file lands
  # somewhere no process in the container can see, and python3 reports it
  # missing while the copy reported success. Measured on 2026-09-21.
  docker cp "$WORK/wire.py" "$CONTAINER:/usr/local/lib/fermix-wire.py" >/dev/null ||
    fail "the wire client could not be copied into the container"
  inside test -f /usr/local/lib/fermix-wire.py ||
    fail "the wire client was copied and is not visible inside the container, so a mount is hiding it"
}

# The narrowest possible statement of "is this the new engine": a module that
# exists only in the newer engine, found in the EXTRACTED payload.
#
# Not the packaged binary. That wrapper is new on disk whether or not it ever
# unpacked anything -- it is new on the owner's machine right now, while the
# BEAM code it runs dates from hours earlier. The extraction is the only place
# the running code can be read from a file.
#
# With a negative control, for the same reason the keyring row has one: a search
# that matches an invented module name is a search that would say yes to
# anything, and then "present" would mean nothing at all.
check_the_extracted_payload_is_the_new_engine() {
  local present absent
  present="$(extracted_beams 'Elixir.FermixCore.Setup.SecretWriter.Migration.beam')"
  absent="$(extracted_beams 'Elixir.FermixCore.NoSuchModule.beam')"

  [ "$absent" = "0" ] ||
    fail "the extracted-payload search matched an invented module, so it would say yes to anything"
  [ "$present" != "0" ] ||
    fail "the extracted payload carries no Migration module: the wrapper on disk is the new one and the code it runs is older"
  echo "  the extracted payload carries the new engine's modules (control: invented name found $absent times)"
}

# Counts matching files under this account's extraction, in BOTH roots it can
# live in. The launcher names the directory after the payload digest under
# ${XDG_CACHE_HOME:-$HOME/.cache}/fermix/runtime; the old default applies
# wherever that variable is unset. Searching only one would report an old
# payload when it is merely somewhere else, which is a gate failing because the
# fix worked.
extracted_beams() {
  local count
  count="$(as_test sh -c "find \"\$HOME/.local/share/.burrito\" \"\${XDG_CACHE_HOME:-\$HOME/.cache}/fermix/runtime\" -name '$1' 2>/dev/null | wc -l || true" |
    tr -d '\r\n')"
  case "$count" in
    "" | *[!0-9]*) echo 0 ;;
    *) echo "$count" ;;
  esac
}

await_engine_after_restart() {
  local attempt
  for attempt in $(seq 1 "$USER_MANAGER_ATTEMPTS"); do
    as_test fermix service status --json >/dev/null 2>&1 && return 0
    sleep 1
  done
  fail "the engine did not answer within ${USER_MANAGER_ATTEMPTS}s of the restart"
}

install_older_package_and_run_it() {
  step "the older engine, installed and running"
  local name
  name="old-$(basename "$UPGRADE_FROM")"
  inside mkdir -p /packages
  docker cp "$UPGRADE_FROM" "$CONTAINER:/packages/$name" ||
    fail "the older package could not be copied into the container"

  if [ "$FAMILY" = "deb" ]; then
    inside apt-get update -qq || fail "the image's package lists could not be read"
    inside apt-get install -y -qq "/packages/$name" ||
      fail "the older package did not install, so there is nothing to upgrade over"
  else
    inside dnf install -y "/packages/$name" ||
      fail "the older package did not install, so there is nothing to upgrade over"
  fi
  start_engine
  echo "  the older engine is running; this row now upgrades underneath it"
}

install_package() {
  step "the install"
  local name
  name="$(basename "$PACKAGE")"
  inside mkdir -p /packages
  docker cp "$PACKAGE" "$CONTAINER:/packages/$name" ||
    fail "the package could not be copied into the container"

  if [ "$FAMILY" = "deb" ]; then
    inside apt-get update -qq || fail "the image's package lists could not be read"
    inside apt-get install -y -qq "/packages/$name" ||
      fail "the package declares a relation $IMAGE cannot satisfy, or did not install"
    inside dpkg -L fermix-desktop > "$EVIDENCE/installed_paths.txt"
  else
    inside dnf install -y "/packages/$name" ||
      fail "the package declares a requirement $IMAGE cannot satisfy, or did not install"
    inside rpm -ql fermix-desktop > "$EVIDENCE/installed_paths.txt"
  fi
  echo "  installed, and its file list is in $EVIDENCE/installed_paths.txt"
}

# Amendment section 3.2. Both halves: the engine's paths, which are at the exact
# place the headless `fermix` package puts them, and the private prefix, which
# is the whole of what this amendment adds.
check_installed_paths() {
  step "what the package installed"
  local path
  for path in \
    /usr/bin/fermix \
    /usr/lib/fermix/cosign \
    /usr/lib/systemd/user/fermix.service \
    /usr/share/fermix/engine.json \
    /usr/lib/fermix-desktop/bin/fermix-desktop \
    /usr/lib/fermix-desktop/lib/gio/modules/libdconfsettings.so \
    /usr/lib/fermix-desktop/lib/gdk-pixbuf-2.0/2.10.0/loaders.cache \
    /usr/lib/fermix-desktop/share/glib-2.0/schemas/gschemas.compiled \
    "/usr/share/applications/$APP_ID.desktop" \
    "/usr/share/metainfo/$APP_ID.metainfo.xml" \
    "/usr/share/dbus-1/services/$APP_ID.service" \
    "/usr/lib/systemd/user/app-$APP_ID.service" \
    "/usr/share/icons/hicolor/256x256/apps/$APP_ID.png" \
    "/usr/share/icons/hicolor/512x512/apps/$APP_ID.png" \
    /usr/share/fermix-desktop/build.json \
    /usr/lib/fermix-desktop/share/fermix-desktop-runtime/identity.json \
    /usr/share/doc/fermix-desktop/copyright; do
    inside test -f "$path" || fail "the package does not install $path"
  done

  # A symbolic link rather than a wrapper script, so that the process the
  # desktop sees is the application itself and `$ORIGIN` resolves through the
  # link target (amendment section 4.3).
  inside test -L /usr/bin/fermix-desktop ||
    fail "/usr/bin/fermix-desktop is not a symbolic link to the private prefix"
  echo "  every path the package claims is there, and the launcher is a link"
}

# The engine stores a channel bot key by shelling out to `secret-tool`, so the
# thing that has to be true after install is that the program resolves on PATH
# -- not that a particular file exists. The distinction is not pedantry: Fedora
# links /usr/sbin to bin, so the same executable answers to both paths, and a
# check written as `test -f /usr/bin/secret-tool` would fail on a row where the
# engine worked perfectly. `command -v` asks the question the engine asks.
#
# It asks it in BOTH of the environments a consumer of the program actually
# has, because neither of them is reliably the stricter one. That was the
# tempting simplification and it is false. Measured with the test account
# lingering and its user manager up, in the smoke images themselves:
#
#   ubuntu:22.04  login and unit IDENTICAL
#   ubuntu:24.04  login and unit IDENTICAL
#   ubuntu:26.04  login and unit IDENTICAL
#                 all three /usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:
#                           /sbin:/bin plus games and snap
#   debian:12     login  /usr/local/bin:/usr/bin:/bin plus games   -- no sbin
#                 unit   /usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
#   debian:13     login  as debian:12
#                 unit   /usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin
#                        -- narrower than debian:12's: no /sbin, no /bin
#   almalinux:9   login  ~/.local/bin:~/bin:/usr/local/bin:/usr/bin:/usr/local/sbin:/usr/sbin
#                 unit   /usr/local/bin:/usr/local/sbin:/usr/bin:/usr/sbin
#   fedora:44     login  ~/.local/bin:~/bin:/usr/local/bin:/usr/bin
#                 unit   /usr/local/bin:/usr/bin
#
# Debian gives the unit MORE than the login shell -- systemd's compiled-in
# default carries sbin where an ordinary session does not. Fedora and AlmaLinux
# give it LESS: the unit's directories are all present in the login shell,
# which adds ~/.local/bin and ~/bin. Ubuntu gives both exactly the same list on
# all three releases, so the difference does not exist there at all. The subset
# therefore runs in opposite directions on different families, and there is no
# single environment that, if it can find the program, guarantees the other can.
#
# One hazard that is not about membership at all: on AlmaLinux the two
# environments hold the SAME directories in a DIFFERENT ORDER -- the unit has
# /usr/local/bin:/usr/local/sbin:/usr/bin:/usr/sbin, the login shell
# /usr/local/bin:/usr/bin:/usr/local/sbin:/usr/sbin. A program installed in both
# /usr/bin and /usr/local/sbin therefore resolves to a DIFFERENT FILE in the two
# environments, and the service and the application would run different
# binaries with no other gate noticing. So the two answers are compared, and a
# genuine mismatch is a refusal rather than a line in a log nobody rereads.
#
# The comparison is by device and inode after following links, not by path.
# A path comparison would refuse on Fedora, where /usr/sbin is a symbolic link
# to bin and the two spellings name one file; `stat -L` resolves first, so one
# file under two names passes and two files under any names do not. Device as
# well as inode because an inode number is only unique within a filesystem, and
# /usr/local is a separate mount often enough for that to matter. Both sides are
# resolved in the same container and the same run, since inode numbers mean
# nothing across containers. If either side finds nothing, that is the
# absent-program failure above rather than a mismatch: the comparison only
# applies when both resolved.
#
# The two consumers are real and distinct: `fermix.service` runs under the
# systemd user manager, and the desktop application gets whatever the session
# hands it. Checking one and inferring the other is how a gate passes on a
# machine where the product does not work. So this asserts both, and says which
# is which when it fails, because "secret-tool not found" means something
# different for a service than for a window.
#
# Nothing today turns on the difference: secret-tool is in /usr/bin on every deb
# row, and Fedora's /usr/sbin is a symbolic link to bin, so both paths name one
# file. It will matter the first time a dependency of ours lands in sbin, or in
# a home directory, on a target where only one of the two environments looks
# there. `su -` rather than `docker exec -u` for the session half, because only
# a login shell builds a session's real PATH and `docker exec` inherits the
# image's ENV, which is a third thing belonging to neither consumer.
# The helpers our own libraries spawn by absolute path are installed and
# runnable on this row.
#
# These are the dependencies no link check can see. GLib is compiled with
# /usr/lib/fermix-desktop/libexec/gio-launch-desktop baked in and spawns it for
# every g_app_info_launch_default_for_uri; dconf-service is started the same
# way through D-Bus activation. Neither is a NEEDED entry, so the relations gate
# -- which holds declared dependencies equal to what the ELFs require -- is
# structurally incapable of noticing either one, and so is every check that
# starts from what the package contains rather than from what it should.
#
# They were absent from every package built before 2026-09-20. The only symptom
# was that opening a URL from the window did nothing at all on an installed
# machine: no error in the journal, no dialog, just the address fallback. The
# owner reported it as the browser never opening, and three separate
# environment theories were investigated before anyone looked for the file.
#
# Executability is checked rather than existence alone, because a helper
# installed without its mode bit fails in exactly the same silent way as one
# that is not installed at all.
check_spawned_helpers_are_installed() {
  step "the helpers our libraries spawn by path"
  local helper

  for helper in gio-launch-desktop dconf-service; do
    inside test -x "/usr/lib/fermix-desktop/libexec/$helper" ||
      fail "/usr/lib/fermix-desktop/libexec/$helper is missing or not executable, so what spawns it fails silently on this row"
    echo "  ok: $helper is installed and executable"
  done
}

check_secret_tool_resolves() {
  step "the secret store's helper, in both environments that look for it"
  local uid unit_path session unit

  session="$(inside su - test -c 'command -v secret-tool' | tr -d '\r')" || session=""
  [ -n "$session" ] ||
    fail "secret-tool does not resolve on the desktop session's PATH, so the application cannot save a channel bot key"

  uid="$(inside id -u test | tr -d '\r')"
  unit_path="$(as_test systemctl --user show-environment |
    tr -d '\r' | sed -n 's/^PATH=//p')"
  [ -n "$unit_path" ] ||
    fail "the user manager for uid $uid reports no PATH, so what fermix.service would search cannot be established"
  unit="$(as_test env "PATH=$unit_path" sh -c 'command -v secret-tool' | tr -d '\r')" || unit=""
  [ -n "$unit" ] ||
    fail "secret-tool does not resolve on the user manager's PATH ($unit_path), so fermix.service cannot save a channel bot key"

  local session_id unit_id
  session_id="$(inside su - test -c "stat -L -c '%d:%i' '$session'" | tr -d '\r')"
  unit_id="$(as_test stat -L -c '%d:%i' "$unit" | tr -d '\r')"
  [ -n "$session_id" ] && [ -n "$unit_id" ] ||
    fail "secret-tool resolved in both environments but could not be stat'd, so the two cannot be compared"

  # The paths are printed whatever the outcome: a path is what a person acts on,
  # and the device:inode pair is only how the decision was reached.
  echo "  the session finds it at $session"
  echo "  fermix.service's own PATH finds it at $unit"
  [ "$session_id" = "$unit_id" ] ||
    fail "the session and fermix.service resolve secret-tool to DIFFERENT files -- session $session ($session_id), unit $unit ($unit_id); the application and the service would run different binaries"
  echo "  both are the same file ($session_id)"
}

# /usr/share/doc is not guaranteed ground. Both package managers can be told to
# throw it away -- dpkg through a path-exclude, rpm through tsflags=nodocs --
# and both go on listing what they discarded, so `dpkg -L` is not evidence that
# a file is on disk. That is a real configuration and not only a container's:
# the official images ship it, and a person can set it on a real machine.
#
# So the runtime manifest is checked where it is guaranteed, in the package
# itself, and nothing load-bearing is asked of it after install. What the
# installed tree is asked for instead is identity.json, which lives under the
# private prefix where no exclusion reaches it.
# Does the installed binary carry the change this build exists to ship?
#
# Every other row here would pass a package built from a tree taken minutes
# before a fix: same version, same file list, same identity, same window. One
# was handed to this gate on 2026-09-20 and did exactly that, while missing the
# bounded retry acceptance rows 18 and 19 promise a reviewer -- so those rows
# would have failed against a promise the binary could not keep, for a reason
# that has nothing to do with the rows.
#
# FIX_STRING is a message the change emits, so it exists in the binary only if
# the change is compiled in. FIX_CONTROL is a string that is there either way,
# and it is the reason this is a check rather than a hope: `strings` is absent
# from debian:13, and a probe that cannot run reports a clean zero for
# everything -- absence of the change and absence of the tool giving one answer
# is the worst shape a check can have. FIX_ABSENT is a near-miss that must NOT
# be found, so a degenerate match that says yes to anything fails too.
FIX_STRING="never got a slot"
FIX_CONTROL="Where keys are stored"
FIX_ABSENT="never got a zlot"

check_binary_carries_the_fix() {
  step "the change this build ships is in the binary"
  local bin=/usr/lib/fermix-desktop/bin/fermix-desktop

  local control found absent
  control="$(binary_occurrences "$bin" "$FIX_CONTROL")"
  [ "$control" != "0" ] ||
    fail "the probe itself is broken: '$FIX_CONTROL' is in every build of this binary and was not found, so a missing fix and a missing tool cannot be told apart"

  absent="$(binary_occurrences "$bin" "$FIX_ABSENT")"
  [ "$absent" = "0" ] ||
    fail "the probe matches a string that is not there ('$FIX_ABSENT'), so it would say yes to anything"

  found="$(binary_occurrences "$bin" "$FIX_STRING")"
  [ "$found" != "0" ] ||
    fail "this package does not carry '$FIX_STRING': it was built before that change landed, and acceptance rows 18 and 19 would fail against a promise it cannot keep"
  echo "  '$FIX_STRING' found $found time(s); control found $control; near-miss found $absent"
}

# One number. `grep -c` prints 0 AND exits 1 when nothing matches, so a
# fallback fires as well and the result becomes two lines -- measured the hard
# way on the keyring row.
binary_occurrences() {
  local count
  count="$(inside sh -c "grep -c -a -F -- '$2' '$1' 2>/dev/null | head -1 || true" | tr -d '\r\n')"
  case "$count" in
    "" | *[!0-9]*) echo 0 ;;
    *) echo "$count" ;;
  esac
}

check_package_carries_the_manifest() {
  local manifest listing name
  manifest=usr/share/doc/fermix-desktop/runtime-manifest.json
  name="$(basename "$PACKAGE")"
  if [ "$FAMILY" = deb ]; then
    listing="$(inside dpkg-deb -c "/packages/$name")" ||
      fail "the deb cannot be listed, so its contents cannot be checked"
  else
    listing="$(inside rpm -qlp "/packages/$name")" ||
      fail "the rpm cannot be listed, so its contents cannot be checked"
  fi
  case "$listing" in
    *"$manifest"*) echo "  the package carries /$manifest, whether or not the host keeps it" ;;
    *) fail "the package carries no /$manifest, so the licence obligation has no answer" ;;
  esac
}

# The postinstall materialises the musl loader every packaged ELF names, and a
# package whose interpreter has no file is a package that installs and cannot
# exec. It is checked here rather than trusted because it is the one thing the
# maintainer script does that nothing else would notice.
# Three probes, not one, and the two extra ones are the point.
#
# This check reads strings out of a binary. Every way that reading can go wrong
# produces the SAME output as the thing it looks for being absent: a file that
# is not there, a file that is text rather than an ELF, a tool missing from the
# image, a path that moved. All of them find nothing, and finding nothing is
# what this check calls a failure -- so without controls it cannot tell "the
# engine names no loader" from "I did not read the engine".
#
# That is not hypothetical. The layout very nearly changed underneath this
# check tonight: /usr/bin/fermix was to become a shell script with the ELF at
# /usr/lib/fermix/engine. Had it landed, this would have reported that the
# packaged engine names no loader, which would have been false and would have
# sent somebody to look at the postinstall. The layout change was withdrawn;
# the lesson was not.
#
#   needle    the musl interpreter, which is what the check is for
#   positive  the file is an ELF. If this fails the check refuses outright and
#             says it could not read the engine, rather than reporting the
#             needle absent
#   invented  a near-miss that must NOT be found, so a hit is not an artefact
#             of the matching
# Every --json command's stdout parses as JSON from its first byte.
#
# No file-level check can see this one. The package can carry exactly the right
# files, every ELF can resolve every library, every declared relation can be
# justified, and `fermix doctor --json` can still emit two lines of chatter
# before the JSON and break every consumer of it.
#
# That is not a hypothetical either: slice2 found precisely this while trying
# the installed deb, when a wrapper that prints before the BEAM starts began
# announcing an overridden install path on every invocation. The archive
# verified, the package verified, and the command was broken. It was caught
# because someone ran the real thing on a real install rather than reading the
# artefact -- which is the only layer this failure is visible from, and so it
# is the layer the check belongs in.
#
# From byte zero, not "contains JSON": a leading blank line, a banner or a
# warning all leave the JSON present and the output unusable, and `grep '{'`
# would pass every one of them.
check_json_commands_emit_only_json() {
  step "the --json commands, from the first byte"
  local command output

  # `service status --json` only. `fermix doctor` takes `--full` and nothing
  # else -- measured, not assumed: it answers "invalid options: [{\"--json\",
  # nil}]" on this build. Doctor's JSON lives behind the management method
  # `doctor.*`, not behind a CLI flag, so asserting a `doctor --json` here tests
  # a command form that cannot exist on any build.
  for command in "service status --json"; do
    # stderr is KEPT. Discarding it turns every failure into "produced nothing
    # at all", which is what this check said while the real message -- that the
    # option was rejected -- was being thrown away.
    output="$(inside su - test -c "fermix $command" 2>&1 | tr -d '\r')" || true

    [ -n "$output" ] ||
      fail "fermix $command produced nothing at all, so there is no JSON to judge"

    printf '%s' "$output" | python3 -c 'import json,sys; json.load(sys.stdin)' 2>/dev/null ||
      fail "fermix $command did not emit JSON from its first byte; every --json consumer of it is broken. First line was: $(printf '%s' "$output" | head -n 1)"

    echo "  ok: fermix $command parses as JSON from byte 0"
  done

  # The control: the assertion above must be capable of failing. A line of
  # chatter in front of real JSON has to be refused, or the check is only
  # testing that python3 exists.
  printf '[i] a banner\n{"ok":true}\n' | python3 -c 'import json,sys; json.load(sys.stdin)' 2>/dev/null &&
    fail "this check accepted a banner in front of JSON, so it proves nothing"
  echo "  ok: a banner before the JSON would be refused"
}

check_loader_store() {
  local engine="/usr/bin/fermix"
  local interpreter invented

  # The positive control, first: everything below is meaningless without it.
  inside test -f "$engine" ||
    fail "there is no $engine to read, so nothing about the loader can be established here"
  inside sh -c "head -c 4 '$engine' | grep -q '^.ELF'" ||
    fail "$engine is not an ELF, so this check cannot read an interpreter out of it: the engine has moved, and this check must move with it rather than reporting a missing loader"

  # `|| true` here is not belt and braces. With `set -o pipefail` the inner
  # grep's exit status becomes the substitution's, so an engine that named no
  # loader would end the run silently -- and the refusal three lines below,
  # which says exactly that, could never print. The check would lose its own
  # error message in the one case it exists to report.
  interpreter="$(inside sh -c \
    "tr -c '[:print:]' '\n' < $engine | grep -m1 '^/var/lib/fermix/runtimes/[0-9a-f]*/libc-musl\.so$' || true" |
    tr -d '\r')"
  case "$interpreter" in
    /var/lib/fermix/runtimes/*/libc-musl.so) ;;
    *) fail "the packaged engine names no loader under /var/lib/fermix/runtimes" ;;
  esac

  # The invented control. A near-miss of the path above, which no engine can
  # contain; if the search finds it, the search is not searching.
  # `|| true` is load-bearing: this grep is SUPPOSED to find nothing, and under
  # `set -e` a command substitution whose pipeline exits non-zero kills the
  # script with no message at all. Without it this check can never pass, and it
  # fails in the one way that teaches nothing -- exit 1 and silence.
  invented="$(inside sh -c \
    "tr -c '[:print:]' '\n' < $engine | grep -m1 '^/var/lib/fermix/runtimez/[0-9a-f]*/libc-musl\.so$' || true" |
    tr -d '\r')"
  [ -z "$invented" ] ||
    fail "a path no engine contains was found in $engine, so this check's reading proves nothing"

  inside test -x "$interpreter" ||
    fail "the postinstall did not materialise $interpreter"
  echo "  the loader store is at $interpreter"
}

# The window compares the id compiled into it with the one in this file, so the
# two have to be one value. A package that ships one and not the other is
# silently unknown rather than wrong, which is why it is checked here.
#
# Read with sed rather than with a JSON parser, because this is a machine a
# person could have rather than a build image: it carries no python, and adding
# one to read one field would be weight the gate does not need.
check_build_identity() {
  step "the packaged binary says what it is"
  local build_id version_line
  build_id="$(inside sed -n 's/.*"build_id"[^"]*"\([^"]*\)".*/\1/p' \
    /usr/share/fermix-desktop/build.json | tr -d '\r')"
  case "$build_id" in
    "" | *[[:space:]]*) fail "the installed manifest names no build id: '$build_id'" ;;
  esac
  echo "  the installed manifest names build $build_id"

  # The build id cannot tell two engines apart: a dirty working-tree build keeps
  # `dev-<commit>-dirty` while the code under it changes, which is exactly how a
  # fixed engine and the engine it fixed can carry the same name. The tree
  # digest is the field that moves, so that is the one this run records.
  local engine_tree
  engine_tree="$(inside sed -n 's/.*"engine_tree_sha256"[^"]*"\([^"]*\)".*/\1/p' \
    /usr/share/fermix-desktop/build.json | tr -d '\r')"
  case "$engine_tree" in
    "" | *[[:space:]]*) fail "the installed manifest names no engine tree digest" ;;
  esac
  echo "  the engine tree it carries is $engine_tree"
  collect /usr/share/fermix-desktop/build.json build.json

  # Versions do not identify a runtime: three different trees all answer "gtk
  # 4.16.7, libadwaita 1.6.9", and a stale prefix once shipped past two guards
  # for exactly that reason. The tree states its own identity instead, and the
  # evidence keeps it so a report about this run names the runtime it ran.
  local cache_key
  cache_key="$(inside sed -n 's/.*"cache_key"[^"]*"\([^"]*\)".*/\1/p' \
    /usr/lib/fermix-desktop/share/fermix-desktop-runtime/identity.json | tr -d '\r')"
  case "$cache_key" in
    [0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]*) ;;
    *) fail "the installed runtime names no cache key: '$cache_key'" ;;
  esac
  echo "  the installed runtime is $cache_key"
  collect /usr/lib/fermix-desktop/share/fermix-desktop-runtime/identity.json \
    runtime_identity.json

  version_line="$(inside /usr/bin/fermix-desktop --version)" ||
    fail "the packaged binary does not run"
  echo "  $version_line"
  case "$version_line" in
    *"(build $build_id)"*) echo "  the binary and the manifest name one build" ;;
    *) fail "the binary reports '$version_line' and the manifest names $build_id" ;;
  esac
}

start_engine() {
  step "the engine, under this account's own user manager"
  inside loginctl enable-linger test || fail "linger could not be granted"

  local uid attempt
  uid="$(inside id -u test | tr -d '\r')"
  echo -n "  waiting for the user manager"
  for attempt in $(seq 1 "$USER_MANAGER_ATTEMPTS"); do
    if inside systemctl is-active "user@$uid.service" >/dev/null 2>&1; then
      echo
      echo "  the user manager is up after ${attempt}s"
      break
    fi
    echo -n "."
    sleep 1
  done
  inside systemctl is-active "user@$uid.service" >/dev/null ||
    fail "this account has no user manager within ${USER_MANAGER_ATTEMPTS}s, so there is nowhere to install a user unit"

  # A home with a space in it, because that is the path shape that breaks a
  # renderer that forgot to quote (M38 section 13.1 gate 29).
  local install_json status_json
  install_json="$(as_test fermix service install --json --home "$HOME_PATH")" ||
    fail "fermix service install refused: $install_json"
  printf '%s\n' "$install_json" > "$EVIDENCE/service_install.json"
  echo "  install: $install_json"
  case "$install_json" in
    *'"ok":true'*) ;;
    *) fail "fermix service install did not report ok" ;;
  esac

  status_json="$(as_test fermix service status --json)" ||
    fail "fermix service status refused: $status_json"
  printf '%s\n' "$status_json" > "$EVIDENCE/service_status.json"
  echo "  status: $status_json"
  check_status "$status_json"
}

# What `service install` promised: aligned, enabled for the next login and
# running now. A smoke that accepted a status saying otherwise would not notice
# an engine whose install reports success and leaves nothing running, which is a
# state this gate has already seen on one systemd version.
check_status() {
  local status_json="$1"
  case "$status_json" in
    *'"alignment":"aligned"'*) ;;
    *) fail "fermix service status does not report the two builds as aligned" ;;
  esac
  case "$status_json" in
    *'"enabled":true'*) ;;
    *) fail "the service is installed and the status does not report it as enabled" ;;
  esac
  case "$status_json" in
    *'"active":true'*) ;;
    *) fail "the service is installed and the status does not report it as active" ;;
  esac
  echo "  the engine is aligned, enabled and running"
}

# The release binary has no capture mode: that is a debug build's, and a package
# ships a release build. So the picture is taken of the display rather than by
# the application, which also proves the one thing the application's own
# renderer could not: the class the display server sees on the window, which is
# the sixth of the six places the application identity lives and the only one
# that can be checked against a running window rather than against a file.
#
# The script is written here and copied in rather than quoted through two
# shells, because a nested quote is how a check like this silently stops
# checking. Everything it writes lives in /evidence, an ordinary directory:
# /tmp inside the container is a tmpfs, and `docker cp` reads the image layer
# underneath a tmpfs rather than the live mount, so a file copied through /tmp
# is a file that is not there.
write_window_script() {
  local path="$1"
  cat > "$path" <<EOF
#!/bin/bash
set -uo pipefail
application_id="$APP_ID"
attempts=$WINDOW_ATTEMPTS
EOF
  cat >> "$path" <<'EOF'

/usr/bin/fermix-desktop &
application=$!

found=
for _ in $(seq 1 "$attempts"); do
  if xdotool search --class "$application_id" >/dev/null 2>&1; then
    found=yes
    break
  fi
  sleep 0.5
done

# A window exists; give the first read of the daemon and the toolkit's own
# transition time to land before the picture is taken.
sleep 3

import -window root /evidence/home_running.png
xdotool search --class "$application_id" getwindowname %@ > /evidence/window.txt 2>/dev/null
xprop -root _NET_CLIENT_LIST > /evidence/clients.txt 2>/dev/null

kill "$application" 2>/dev/null
wait "$application" 2>/dev/null

[ -n "$found" ]
EOF
  chmod +x "$path"
}

# A session bus of this run's own, because a GtkApplication takes its name on
# the bus and a window that could not register is a window that never draws.
# M38 section 13.1 names `dbus-run-session` for exactly this. Nothing on that
# bus outlives the check.
draw_window_on_x11() {
  step "the window, on an X display"
  local script status
  script="$WORK/window_check.sh"
  write_window_script "$script"
  inside mkdir -p /evidence
  inside chown test:test /evidence
  docker cp "$script" "$CONTAINER:/evidence/window_check.sh" >/dev/null
  inside chmod 0755 /evidence/window_check.sh

  set +e
  docker exec -u test \
    -e "XDG_RUNTIME_DIR=/run/user/$(inside id -u test | tr -d '\r')" \
    -e HOME=/home/test \
    "$CONTAINER" \
    dbus-run-session -- \
    xvfb-run -a --server-args="-screen 0 1280x800x24" /evidence/window_check.sh
  status=$?
  set -e

  [ "$status" -eq 0 ] ||
    fail "no window carrying the class $APP_ID appeared within $((WINDOW_ATTEMPTS / 2)) seconds"
  echo "  a window carrying the class $APP_ID appeared"
  echo "  its title: $(inside cat /evidence/window.txt 2>/dev/null || echo unknown)"
  collect /evidence/home_running.png home_running.png
  collect /evidence/window.txt window.txt
  collect /evidence/clients.txt clients.txt
}

# What Xvfb cannot prove. The private libwayland-client is the one component
# this design replaces at a version the host does not have, and its failure mode
# is a window that never appears rather than a link error, so an X11 run stays
# green straight through it.
#
# There is no window manager on a headless Weston and no way to ask it for a
# window class, so the assertion is a different one and it is stated rather than
# implied: GDK_BACKEND=wayland makes a failure to reach the compositor fatal
# instead of a silent fall back to X, the application is still alive after it
# has had time to draw, and the compositor's own screenshot differs from the one
# taken before the application started.
write_wayland_script() {
  local path="$1"
  cat > "$path" <<EOF
#!/bin/bash
set -uo pipefail
attempts=$WESTON_ATTEMPTS
EOF
  cat >> "$path" <<'EOF'

export WAYLAND_DISPLAY=wayland-smoke
socket="$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY"

# Weston 13 renamed the backend from `headless-backend.so` to `headless`. Both
# spellings are tried, once each, and a compositor that starts under neither is
# a refusal naming both rather than a wait that never ends.
#
# Two flags here are not decoration. The headless backend's default renderer
# draws nothing at all, so the capture protocol hands the screenshooter a
# zero-sized frame and it dies on an assertion; --renderer=pixman is the
# renderer that composites without a GPU. --debug is what authorises any client
# to bind the capture protocol -- without it the screenshooter is refused as
# unauthorised. Both were established against weston 14.0.2 headless.
compositor=
for backend in headless headless-backend.so; do
  weston --backend="$backend" --renderer=pixman --width=1280 --height=800 \
    --socket="$WAYLAND_DISPLAY" --idle-time=0 --debug \
    >> /evidence/weston.log 2>&1 &
  candidate=$!
  for _ in $(seq 1 "$attempts"); do
    [ -S "$socket" ] && break
    sleep 0.5
  done
  if [ -S "$socket" ]; then
    compositor=$candidate
    break
  fi
  kill "$candidate" 2>/dev/null
  wait "$candidate" 2>/dev/null
done

if [ -z "$compositor" ]; then
  echo "weston started under neither --backend=headless nor --backend=headless-backend.so" >&2
  exit 1
fi

screenshot() {
  rm -f wayland-screenshot-*.png
  weston-screenshooter 2>>/evidence/weston.log || return 1
  mv -f wayland-screenshot-*.png "$1" || return 1
}

screenshot /evidence/wayland_before.png ||
  { echo "the compositor took no first screenshot" >&2; exit 1; }

export GDK_BACKEND=wayland
/usr/bin/fermix-desktop &
application=$!
sleep 8

alive=
kill -0 "$application" 2>/dev/null && alive=yes
screenshot /evidence/wayland_running.png
taken=$?

kill "$application" 2>/dev/null
wait "$application" 2>/dev/null
kill "$compositor" 2>/dev/null
wait "$compositor" 2>/dev/null

[ -n "$alive" ] || { echo "the application exited under GDK_BACKEND=wayland" >&2; exit 1; }
[ "$taken" -eq 0 ] || { echo "the compositor took no screenshot with the window open" >&2; exit 1; }
if cmp -s /evidence/wayland_before.png /evidence/wayland_running.png; then
  echo "the compositor sees the same screen with the window as without it" >&2
  exit 1
fi
EOF
  chmod +x "$path"
}

draw_window_on_wayland() {
  step "the window, on a headless Weston"
  local script status uid
  script="$WORK/wayland_check.sh"
  write_wayland_script "$script"
  docker cp "$script" "$CONTAINER:/evidence/wayland_check.sh" >/dev/null
  inside chmod 0755 /evidence/wayland_check.sh
  uid="$(inside id -u test | tr -d '\r')"

  set +e
  docker exec -u test -w /evidence \
    -e "XDG_RUNTIME_DIR=/run/user/$uid" \
    -e HOME=/home/test \
    "$CONTAINER" \
    dbus-run-session -- /evidence/wayland_check.sh
  status=$?
  set -e

  docker cp "$CONTAINER:/evidence/weston.log" "$EVIDENCE/weston.log" >/dev/null 2>&1 || true
  [ "$status" -eq 0 ] ||
    fail "the window did not draw on Wayland; $EVIDENCE/weston.log says what the compositor saw"
  echo "  the compositor's screen changed when the window opened"
  collect /evidence/wayland_before.png wayland_before.png
  collect /evidence/wayland_running.png wayland_running.png
}

collect() {
  docker cp "$CONTAINER:$1" "$EVIDENCE/$2" >/dev/null 2>&1 ||
    fail "the run produced no $1"
  echo "  $EVIDENCE/$2"
}

main() {
  parse_arguments "$@"
  FAMILY="$(family_of "$IMAGE")" || exit 1
  guard

  TAG="fermix-desktop-smoke-$(printf '%s' "$IMAGE" | tr ':/' '--')"
  EVIDENCE="$ROOT_DIR/packaging/out/smoke/$(printf '%s' "$IMAGE" | tr ':/' '--')"
  WORK="$(mktemp -d "${TMPDIR:-/tmp}/install-smoke.XXXXXX")"
  trap cleanup EXIT
  rm -rf -- "$EVIDENCE"
  mkdir -p "$EVIDENCE"

  start_machine
  [ -z "$UPGRADE_FROM" ] || install_older_package_and_run_it
  install_package
  check_installed_paths
  [ -z "$UPGRADE_FROM" ] || check_the_upgrade_replaced_the_engine
  check_binary_carries_the_fix
  check_secret_tool_resolves
  check_spawned_helpers_are_installed
  check_package_carries_the_manifest
  check_loader_store
  check_json_commands_emit_only_json
  check_build_identity
  start_engine
  draw_window_on_x11
  [ "$WAYLAND" = "0" ] || draw_window_on_wayland

  step "what ran"
  echo "  $IMAGE installed fermix-desktop through $FAMILY, the file list is what"
  echo "  the package claims, the loader store was materialised, the binary and"
  echo "  the manifest name one build, the engine is running under an ordinary"
  echo "  account, and a window with the application's own class was drawn."
  if [ "$WAYLAND" = "0" ]; then
    echo
    echo "install_smoke: what could not run"
    echo "  the Wayland backend, because this row was not asked for --wayland"
  fi
  echo
  echo "install_smoke: ok"
}

main "$@"

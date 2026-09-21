#!/bin/bash
# The keyring row: four arms, one package, one container each.
#
# It exists because of one measured failure. With an unlocked keyring, a
# successful store returned `{:ok, ""}` to a write log that had no clause for
# it, so `secret.set` raised, the caller was told the save had failed, and the
# credential was in the keyring all along. The daemon never died. That shape --
# the success causing the failure, and the failure being invisible to every
# liveness check -- is what this row watches for.
#
# Four arms, because four different machines have to give four answers, and
# collapsing any two of them is the defect this work exists to remove -- the
# owner met a locked keyring reporting an absence, and could not save a key:
#
#   no Secret Service at all   refuse with `unavailable`, never `locked`;
#   unlocked keyring           SUCCEED -- a result rather than an error
#                              envelope, no route failure in the journal, the
#                              value readable back, the daemon unchanged;
#   locked collection          refuse with `locked`, raise no dialog when no
#                              unlock was asked for, and store nothing in
#                              EITHER store;
#   service, no collection     refuse with `unavailable`, never `locked`.
#
# What it still cannot do is the dialog: a container has nobody to read it, so
# acceptance rows 16 to 19 stay human.
set -uo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# Bounds, declared rather than discovered. Each is a count of one-second waits,
# and reaching one is a refusal naming what never arrived.
SYSTEMD_ATTEMPTS=60
USER_MANAGER_ATTEMPTS=30
KEYRING_ATTEMPTS=20

# What the locked arm does with the shim's log. `record` prints it as evidence;
# `refuse` fails the arm when the engine ran `secret-tool store` on a collection
# it had already measured as locked.
#
# It is `refuse` because the engine promises it, confirmed 2026-09-20 and pinned
# on their side by a test named "a locked collection runs no helper at all, so
# its exit cannot decide the answer", which injects a runner that fails if it is
# called and asserts the refusal still comes back as keyring_locked. The reason
# it is a promise rather than an implementation detail is the prompter: on a
# locked collection the helper blocks where a display exists and exits 1 where
# one does not, so consulting it would answer two different things about one
# machine state. Once the property says locked, asking the helper could only
# make the answer worse.
SHIM_VERDICT="refuse"
SHIM_LOG="/home/test/secret-tool-invocations.log"

IMAGE="ubuntu:24.04"
# The engine this package is supposed to carry, as a tree digest. Optional: left
# empty the row reports what it found and runs anyway. Given, it refuses before
# spending a container on the wrong engine.
EXPECT_TREE=""
# The package digest this run was told to test. Empty means unpinned. The tree
# digest cannot stand in for it: two builds minutes apart carry the same engine
# and differ only in app code.
EXPECT_PACKAGE=""
PACKAGE=""
ARM="all"
KEEP=0
CONTAINER=""
EVIDENCE=""
UID_TEST=""
HOME_PATH="/home/test/fermix home"
SECRET_ID="anthropic_api_key"
SECRET_VALUE="sk-keyring-smoke-probe"
# A second id for the consent step, so the file store holds exactly one value
# and "the file copy is gone" is a count rather than an inference.
FILE_SECRET_ID="telegram_bot_token"

fail() {
  echo >&2
  echo "keyring_smoke: $1" >&2
  cleanup
  exit 1
}

step() {
  echo
  echo "keyring_smoke: == $1"
}

cleanup() {
  [ -n "$CONTAINER" ] || return 0
  if [ "$KEEP" = "1" ]; then
    echo "keyring_smoke: $CONTAINER is still running, as asked"
    return 0
  fi
  docker rm -f "$CONTAINER" > /dev/null 2>&1
  CONTAINER=""
}

parse_arguments() {
  local positional=()
  while [ "$#" -gt 0 ]; do
    case "$1" in
      --image)
        [ "$#" -ge 2 ] || fail "--image needs an image name"
        IMAGE="$2"
        shift 2
        ;;
      --arm)
        [ "$#" -ge 2 ] || fail "--arm needs none, keyring, locked, empty or all"
        ARM="$2"
        shift 2
        ;;
      --expect-package)
        [ "$#" -ge 2 ] || fail "--expect-package needs a sha256"
        EXPECT_PACKAGE="$2"
        shift 2
        ;;
      --expect-tree)
        [ "$#" -ge 2 ] || fail "--expect-tree needs an engine tree sha256"
        EXPECT_TREE="$2"
        shift 2
        ;;
      --keep)
        KEEP=1
        shift
        ;;
      -*) fail "unknown option $1" ;;
      *)
        positional+=("$1")
        shift
        ;;
    esac
  done

  [ "${#positional[@]}" -eq 1 ] ||
    fail "usage: keyring_smoke.sh [--image IMAGE] [--arm none|keyring|locked|empty|all] [--expect-tree SHA256] [--keep] PACKAGE"
  PACKAGE="${positional[0]}"

  case "$ARM" in
    none | keyring | locked | empty | all | both) ;;
    *) fail "there is no arm called '$ARM': it is none, keyring, locked, empty or all" ;;
  esac

  [ -f "$PACKAGE" ] || fail "$PACKAGE is not a file"

  if [ -n "$EXPECT_PACKAGE" ]; then
    local actual
    actual="$(sha256sum "$PACKAGE" | cut -d' ' -f1)"
    [ "$actual" = "$EXPECT_PACKAGE" ] ||
      fail "this is not the package this run was told to test: $PACKAGE hashes to $actual, expected $EXPECT_PACKAGE"
    echo "keyring_smoke: the package is $actual, which is the one this run was told to test"
  fi

  case "$PACKAGE" in
    *.deb) ;;
    *.rpm) fail "this row covers the deb family only; $PACKAGE is an rpm" ;;
    *) fail "$PACKAGE is neither a deb nor an rpm" ;;
  esac

  case "$IMAGE" in
    ubuntu:* | debian:*) ;;
    *) fail "this row has no package manager for $IMAGE: it covers ubuntu and debian" ;;
  esac

  command -v docker > /dev/null || fail "docker is not on this host"
}

inside() {
  docker exec "$CONTAINER" "$@"
}

# `--arm all` runs every arm; `both` is kept because it is what the first
# version of this row called the pair it had.
wanted() {
  case "$ARM" in
    all) return 0 ;;
    both) [ "$1" = "none" ] || [ "$1" = "keyring" ] ;;
    *) [ "$ARM" = "$1" ] ;;
  esac
}

# As the ordinary account, on the bus its user manager owns. That bus is the
# point: the engine's unit and the keyring daemon have to share one session, or
# the arm proves only that two unrelated processes were running.
as_test() {
  docker exec -u test \
    -e "XDG_RUNTIME_DIR=/run/user/$UID_TEST" \
    -e HOME=/home/test \
    -e "DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/$UID_TEST/bus" \
    "$CONTAINER" "$@"
}

# The two arms need two machines, and that is the honest shape rather than an
# inconvenience. A machine with gnome-keyring INSTALLED has
# org.freedesktop.secrets as a D-Bus activatable name whether or not a daemon is
# running, so "no Secret Service" cannot be demonstrated on it -- the absent case
# is a machine where nothing implements the interface at all, which is what row
# 21 of the acceptance runbook describes and what a headless server is.
start_machine() {
  local keyring="$1"
  step "the machine: $IMAGE, keyring installed: $keyring"
  local tag state attempt
  tag="fermix-desktop-keyring$keyring-$(printf '%s' "$IMAGE" | tr ':/' '--')"
  docker build --quiet \
    --file "$ROOT_DIR/packaging/docker/Dockerfile.smoke" \
    --build-arg "BASE=$IMAGE" --build-arg "KEYRING=$keyring" \
    --tag "$tag" "$ROOT_DIR" > /dev/null ||
    fail "the container for $IMAGE could not be built"

  CONTAINER="$tag-$$"
  docker run -d --name "$CONTAINER" --privileged --cgroupns=host \
    --tmpfs /run -v /sys/fs/cgroup:/sys/fs/cgroup:rw "$tag" /sbin/init > /dev/null ||
    fail "the container would not start"

  state=""
  for attempt in $(seq 1 "$SYSTEMD_ATTEMPTS"); do
    state="$(inside systemctl is-system-running 2> /dev/null | tr -d '\r')"
    case "$state" in running | degraded) break ;; esac
    sleep 1
  done
  case "$state" in
    running | degraded) echo "  systemd is $state after ${attempt}s" ;;
    *) fail "systemd never came up inside $IMAGE (last state: ${state:-none})" ;;
  esac
}

# The absent arm installs with recommendations refused, and that is not a
# convenience. The package RECOMMENDS gnome-keyring, apt honours
# recommendations by default, and an installed gnome-keyring makes
# org.freedesktop.secrets a D-Bus activatable name whether or not a daemon runs
# -- so the default install cannot demonstrate a machine with no Secret Service
# at all. Refusing the recommendation is also the real configuration this arm
# describes: a server, or a KDE or KeePassXC desktop that carries no
# gnome-keyring. The required relation on secret-tool is unaffected.
install_package() {
  local recommends="$1"
  step "the install (recommendations: $recommends)"
  local name flags
  name="$(basename "$PACKAGE")"
  flags=""
  [ "$recommends" = "1" ] || flags="--no-install-recommends"
  inside mkdir -p /packages
  docker cp "$PACKAGE" "$CONTAINER:/packages/$name" > /dev/null ||
    fail "the package could not be copied into the container"
  inside apt-get update -qq || fail "the image's package lists could not be read"
  # shellcheck disable=SC2086 # one optional flag, deliberately unquoted
  inside apt-get install -y -qq $flags "/packages/$name" ||
    fail "the package declares a relation $IMAGE cannot satisfy, or did not install"
  echo "  installed"
}

# Which engine is actually in this package, read from the installed build.json
# before a single arm runs.
#
# `engine_build_id` cannot do this job: it is identical across engines that
# differ, which I concluded a package predated a fix from -- right answer, wrong
# reason. The tree digest is what separates them. So when the row is told which
# engine it is testing and finds another, it says so here rather than failing
# four arms one at a time and leaving someone to infer why.
engine_in_this_package() {
  local tree
  tree="$(inside sed -n 's/.*"engine_tree_sha256"[: ]*"\([0-9a-f]*\)".*/\1/p' \
    /usr/share/fermix-desktop/build.json | tr -d '\r')"
  [ -n "$tree" ] ||
    fail "this package's build.json names no engine_tree_sha256, so which engine it carries cannot be read"
  printf '%s\n' "$tree" > "$EVIDENCE/engine_tree_sha256.txt"
  echo "  the package carries engine tree $tree"

  [ -n "$EXPECT_TREE" ] || return 0
  [ "$tree" = "$EXPECT_TREE" ] ||
    fail "this package carries engine tree $tree and was expected to carry $EXPECT_TREE: it is the packaging that is wrong, not the engine"
  echo "  which is the engine this run was told to test"
}

start_engine() {
  step "the engine, under this account's own user manager"
  inside loginctl enable-linger test || fail "linger could not be granted"
  UID_TEST="$(inside id -u test | tr -d '\r')"

  local attempt
  for attempt in $(seq 1 "$USER_MANAGER_ATTEMPTS"); do
    inside systemctl is-active "user@$UID_TEST.service" > /dev/null 2>&1 && break
    sleep 1
  done
  inside systemctl is-active "user@$UID_TEST.service" > /dev/null ||
    fail "this account has no user manager within ${USER_MANAGER_ATTEMPTS}s"

  local envelope
  envelope="$(as_test fermix service install --json --home "$HOME_PATH")" ||
    fail "fermix service install refused: $envelope"
  case "$envelope" in
    *'"ok":true'*) echo "  the engine is installed and running" ;;
    *) fail "fermix service install did not report ok: $envelope" ;;
  esac
}

# A logging `secret-tool` earlier on the PATH than the real one. The unit's PATH
# on ubuntu ranks /usr/local/bin second and /usr/bin fourth, verified rather than
# assumed, so the engine's own invocations come through here and are recorded.
# It execs the real binary, so the arm it instruments still behaves normally.
install_shim() {
  local path="$WORK/secret-tool-shim"
  cat > "$path" <<EOF
#!/bin/sh
printf '%s\n' "\$*" >> "$SHIM_LOG"
exec /usr/bin/secret-tool "\$@"
EOF
  docker cp "$path" "$CONTAINER:/usr/local/bin/secret-tool" > /dev/null
  inside chmod 0755 /usr/local/bin/secret-tool
  inside touch "$SHIM_LOG"
  inside chown test:test "$SHIM_LOG"
}

# Emptied immediately before the call under test, because the arms write to the
# keyring themselves while setting up -- locking one requires first creating and
# filling it -- and those writes go through the same shim. The first run of this
# counted the arm's own preparation as the engine's invocation, which is the
# false positive the shim exists to avoid.
reset_shim_log() {
  as_test sh -c ": > '$SHIM_LOG'"
}

# One number, and it has to be one number. `grep -c` prints 0 AND exits 1 when
# nothing matches, so an `|| echo 0` fallback fires as well and the result is
# two lines -- which then compares unequal to "0" and fails an arm that was
# passing. Measured the hard way on 2026-09-20.
shim_store_lines() {
  local count
  count="$(as_test sh -c "grep -c '^store' '$SHIM_LOG' 2>/dev/null | head -1" | tr -d '\r\n')"
  case "$count" in
    "" | *[!0-9]*) echo 0 ;;
    *) echo "$count" ;;
  esac
}

# The management socket speaks 4-byte length-prefixed JSON, which is a client
# rather than a command. This is that client, written into the container once.
write_wire_client() {
  local path="$WORK/wire.py"
  cat > "$path" <<'PY'
import json, socket, struct, sys

sock_path, method, params = sys.argv[1], sys.argv[2], sys.argv[3]
body = json.dumps({"request_id": "keyring-smoke",
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
PY
  docker cp "$path" "$CONTAINER:/tmp/wire.py" > /dev/null
}

call() {
  as_test python3 /tmp/wire.py "$HOME_PATH/daemon.sock" "$1" "$2"
}

# pid and restart_count before, and after. Necessary and NOT sufficient: the
# measured failure left both unchanged, which is exactly why the response and
# the journal are checked too.
daemon_identity() {
  as_test fermix service status --json |
    tr ',' '\n' |
    sed -n 's/.*"pid":"\{0,1\}\([0-9]*\)"\{0,1\}.*/pid=\1/p;s/.*"restart_count":\([0-9]*\).*/restarts=\1/p' |
    sort -u |
    tr '\n' ' '
}

journal_route_failures() {
  as_test journalctl --user -u fermix --no-pager -o cat |
    grep -c "Management route failed" |
    tr -d '\r'
}

# The row the Settings pane draws, read off the wire.
#
# Every other assertion in this file is about what `secret.set` ANSWERS, and the
# app never draws that: it draws a row built from `setup.state.get`. The two can
# disagree without either looking broken. The store field moved out of the top
# level into a nested `secrets` object, an app build went on reading the old
# shape, and the row drew nothing while the engine answered perfectly -- a row
# that says nothing looks exactly like a row nobody checked.
#
# So this asserts the shape and the value, which is the half a container can
# see. What it still cannot see is the window: whether the app reads this row,
# and what it puts on the screen, is acceptance row 18 and stays human.
#
# Two fields, deliberately not one. `store` is where this home saves -- keyring
# until somebody consents to a file, and never "none", because a home that can
# store nothing still saves TO the keyring and the save simply refuses.
# `availability` is whether it could save right now. Collapsing them is what
# made a locked keyring report an absence on the owner's desktop.
state_secrets_row() {
  local want_store="$1" want_availability="$2" label="$3"
  local state row
  state="$(call setup.state.get '{}')"
  printf '%s\n' "$state" > "$EVIDENCE/${label}_state.json"

  row="$(printf '%s' "$state" | sed -n 's/.*"secrets":{\([^}]*\)}.*/\1/p')"
  [ -n "$row" ] ||
    fail "setup.state.get carries no nested secrets row, so the Settings row has nothing to draw: $state"

  case "$row" in
    *"\"store\":\"$want_store\""*) ;;
    *) fail "the Settings row should say store=$want_store and says: {$row}" ;;
  esac
  case "$row" in
    *"\"availability\":\"$want_availability\""*) ;;
    *) fail "the Settings row should say availability=$want_availability and says: {$row}" ;;
  esac
  echo "  the Settings row reads store=$want_store, availability=$want_availability"
}

arm_without_a_secret_service() {
  step "arm one: no Secret Service at all"
  local listed response before after failures
  listed="$(as_test busctl --user list 2> /dev/null | grep -c org.freedesktop.secrets | tr -d '\r')"
  [ "$listed" = "0" ] ||
    fail "something already owns org.freedesktop.secrets, so this arm proves nothing"
  echo "  nothing owns org.freedesktop.secrets"

  before="$(daemon_identity)"
  response="$(call secret.set "{\"id\":\"$SECRET_ID\",\"value\":\"$SECRET_VALUE\"}")"
  printf '%s\n' "$response" > "$EVIDENCE/none_secret_set.json"
  echo "  $response"

  case "$response" in
    *'"code":"secret_store_failed"'*) ;;
    *) fail "with no keyring the store must refuse with secret_store_failed: $response" ;;
  esac
  case "$response" in
    *'"reason":"unavailable"'*) ;;
    *) fail "with no keyring the reason must be unavailable, not whatever this is: $response" ;;
  esac
  case "$response" in
    *'"reason":"locked"'*) fail "an absent store reported itself locked" ;;
  esac

  after="$(daemon_identity)"
  [ "$before" = "$after" ] ||
    fail "the daemon changed across a refused store: $before -> $after"
  failures="$(journal_route_failures)"
  [ "$failures" = "0" ] ||
    fail "the journal records $failures management route failure(s) on the refusing path"
  echo "  refused as unavailable, daemon unchanged ($after)"
  state_secrets_row keyring unavailable none
}

start_keyring() {
  local locked
  as_test sh -c 'echo keyring-smoke | gnome-keyring-daemon --unlock --components=secrets' \
    > /dev/null 2>&1
  await_secret_service

  locked="$(collection_locked)"
  printf '%s\n' "$locked" > "$EVIDENCE/keyring_locked.txt"
  case "$locked" in
    *false*) echo "  a Secret Service is on the engine's bus and its collection is unlocked" ;;
    *) fail "the collection is not unlocked, so this arm would prove the locked path: $locked" ;;
  esac
}

# Waits for a Secret Service to own the name, and answers nothing about its
# collections. The three keyring arms differ only in what they do before this.
await_secret_service() {
  local attempt owner=0
  for attempt in $(seq 1 "$KEYRING_ATTEMPTS"); do
    owner="$(as_test busctl --user list 2> /dev/null | grep -c org.freedesktop.secrets | tr -d '\r')"
    [ "$owner" != "0" ] && return 0
    sleep 1
  done
  fail "no Secret Service appeared on the engine's own bus within ${KEYRING_ATTEMPTS}s"
}

collection_locked() {
  as_test busctl --user get-property org.freedesktop.secrets \
    /org/freedesktop/secrets/collection/login \
    org.freedesktop.Secret.Collection Locked 2>&1 | tr -d '\r'
}

default_alias() {
  as_test busctl --user call org.freedesktop.secrets /org/freedesktop/secrets \
    org.freedesktop.Secret.Service ReadAlias s default 2>&1 | tr -d '\r'
}

# The fingerprint-login shape, headless. A keyring has to exist before it can be
# locked, so one is created and written to, that daemon is stopped BY USER --
# a pattern kill takes PID 1 in a container and loses the session -- and a new
# one is started without --unlock. The collection then exists and is locked.
#
# **It is not the whole of the machine the owner had, and the difference is the
# prompter.** Measured here on 2026-09-20: with a locked collection and no
# display, `secret-tool store` exits 1 at once with "Cannot create an item in a
# locked collection", because gcr-prompter cannot start -- the session bus logs
# `Activated service 'org.gnome.keyring.SystemPrompter' failed`. On a real
# desktop that prompter DOES start, puts a password dialog on the screen, and
# secret-tool blocks on it until someone answers or the caller gives up, which
# is why the owner's own machine produced a timeout rather than a clean refusal.
#
# So this arm exercises the non-blocking half of `locked`. An engine that reads
# the collection's `Locked` property before writing answers the same either way,
# which is the point of reading it; an engine that infers the state from how the
# helper behaved does not, and this arm cannot tell you which one you have. The
# blocking half needs a display and a person: acceptance rows 16 and 17.
lock_the_keyring() {
  as_test sh -c 'echo keyring-smoke | gnome-keyring-daemon --unlock --components=secrets' \
    > /dev/null 2>&1
  await_secret_service
  as_test sh -c 'echo prior-value | secret-tool store --label=prior service fermix-prior account fermix-prior' \
    > /dev/null 2>&1 ||
    fail "the keyring could not be written to, so there is nothing to lock"
  as_test pkill -u test gnome-keyring-d > /dev/null 2>&1
  sleep 2
  as_test sh -c 'gnome-keyring-daemon --start --components=secrets >/dev/null 2>&1' \
    > /dev/null 2>&1
  await_secret_service
}

arm_with_a_locked_keyring() {
  step "arm three: a locked keyring, and no unlock asked for"
  local locked response before after found leftovers
  lock_the_keyring

  locked="$(collection_locked)"
  printf '%s\n' "$locked" > "$EVIDENCE/locked_property.txt"
  case "$locked" in
    *true*) echo "  the login collection exists and is locked" ;;
    *) fail "this arm needs a locked collection and does not have one: $locked" ;;
  esac

  before="$(daemon_identity)"
  reset_shim_log
  response="$(call secret.set "{\"id\":\"$SECRET_ID\",\"value\":\"$SECRET_VALUE\"}")"
  printf '%s\n' "$response" > "$EVIDENCE/locked_secret_set.json"
  # Collected BEFORE the assertions, so an arm that refuses still leaves behind
  # what it saw. The first version of this gathered it afterwards and a failing
  # arm produced an empty file, which is the least useful moment to have no
  # evidence.
  local stores
  stores="$(shim_store_lines)"
  as_test cat "$SHIM_LOG" > "$EVIDENCE/locked_shim_log.txt" 2>/dev/null
  echo "  $response"
  echo "  the helper was invoked with a store $stores time(s)"

  case "$response" in
    *'"code":"secret_store_failed"'*) ;;
    *) fail "a locked keyring must refuse with secret_store_failed: $response" ;;
  esac
  case "$response" in
    *'"reason":"locked"'*) echo "  refused as locked, which is the true reason" ;;
    *'"reason":"unavailable"'*) fail "a locked collection reported itself unavailable: $response" ;;
    *) fail "a locked keyring answered with neither locked nor unavailable: $response" ;;
  esac

  # A refusal that stored the value anyway has happened here on the unlocked
  # path, so it is checked rather than assumed: unlock the collection again and
  # look, and look in the file store too.
  as_test sh -c 'echo keyring-smoke | gnome-keyring-daemon --unlock --components=secrets' \
    > /dev/null 2>&1
  sleep 2
  found="$(as_test secret-tool search --all service fermix 2>&1 | tr -d '\r')"
  printf '%s\n' "$found" > "$EVIDENCE/locked_keyring_contents.txt"
  case "$found" in
    *"$SECRET_VALUE"*) fail "the store was refused and the value is in the keyring anyway" ;;
  esac
  leftovers="$(as_test sh -c "ls -A '$HOME_PATH/secrets' 2>/dev/null | wc -l" | tr -d '\r')"
  [ "$leftovers" = "0" ] ||
    fail "the store was refused and $leftovers file(s) appeared in the file store"
  echo "  nothing was stored, in either store"

  # Did the engine run the helper at all? A refusal read from the collection's
  # `Locked` property never reaches `secret-tool store`; a refusal that came
  # from the helper's own exit code does. The envelope cannot tell them apart.
  #
  # **The count is the weakest thing in this file, which is why the log is kept
  # beside it.** This instrument has already produced, once, the exact artefact
  # it exists to detect: locking a collection means first creating and filling
  # it, that setup write went through the shim, and it was counted as the
  # engine's invocation. A store line here is the engine's only because
  # `reset_shim_log` runs immediately before the call under test and nothing
  # else can have written since. Anyone extending this will reach for the
  # count; read $EVIDENCE/locked_shim_log.txt and see whose invocations they
  # are.
  case "$SHIM_VERDICT" in
    refuse)
      [ "$stores" = "0" ] ||
        fail "the collection was measured as locked and the engine ran secret-tool store $stores time(s) anyway"
      echo "  the engine never ran the helper: the refusal came from the property"
      ;;
    *)
      echo "  the helper was invoked with a store $stores time(s) (recorded, not asserted:"
      echo "  see SHIM_VERDICT). Zero means the refusal came from the measured property."
      ;;
  esac

  after="$(daemon_identity)"
  [ "$before" = "$after" ] ||
    fail "the daemon changed across a locked refusal: $before -> $after"
  echo "  daemon unchanged ($after)"
  # Locked, not absent, and the row has to say so: this is the exact line the
  # owner read as "no keyring" when their keyring was merely shut.
  state_secrets_row keyring locked locked
}

# A Secret Service that is up with no keyring ever created: ReadAlias answers
# "/" and there is no default collection at all. Not absent, not locked, not
# unlocked -- a fourth machine, and the one a KeePassXC user who has not made a
# database yet is sitting on. The contract says it is `unavailable`.
#
# This arm has no differing control available: the engine that has the fix and
# the engine that does not both answer `unavailable` here, so a pass cannot
# prove the arm would notice a change. It was instead proven by deliberate
# failure -- on 2026-09-20, against package a8e37bfc, a copy of this arm
# demanding `locked` refused with "CONTROL: demanded locked and did not get it"
# and the real `unavailable` beside it. So the arm reads the reason rather than
# reporting whatever it finds.
arm_with_no_default_collection() {
  step "arm four: a Secret Service with no default collection"
  local alias response before after
  as_test sh -c 'gnome-keyring-daemon --start --components=secrets >/dev/null 2>&1' \
    > /dev/null 2>&1
  await_secret_service

  alias="$(default_alias)"
  printf '%s\n' "$alias" > "$EVIDENCE/no_collection_alias.txt"
  case "$alias" in
    *'"/"'*) echo "  a Secret Service is up and has no default collection" ;;
    *) fail "this arm needs a service with no default collection: $alias" ;;
  esac

  before="$(daemon_identity)"
  response="$(call secret.set "{\"id\":\"$SECRET_ID\",\"value\":\"$SECRET_VALUE\"}")"
  printf '%s\n' "$response" > "$EVIDENCE/no_collection_secret_set.json"
  echo "  $response"

  case "$response" in
    *'"code":"secret_store_failed"'*) ;;
    *) fail "with no default collection the store must refuse: $response" ;;
  esac
  case "$response" in
    *'"reason":"unavailable"'*) echo "  refused as unavailable, which is what no collection means" ;;
    *'"reason":"locked"'*) fail "a service with no collection reported itself locked: $response" ;;
    *) fail "with no default collection the reason must be unavailable: $response" ;;
  esac

  after="$(daemon_identity)"
  [ "$before" = "$after" ] ||
    fail "the daemon changed across a refused store: $before -> $after"
  echo "  daemon unchanged ($after)"
  state_secrets_row keyring unavailable no_collection
}

arm_with_an_unlocked_keyring() {
  step "arm two: an unlocked keyring, ordinary save"
  local response before after failures found
  start_keyring

  before="$(daemon_identity)"
  reset_shim_log
  response="$(call secret.set "{\"id\":\"$SECRET_ID\",\"value\":\"$SECRET_VALUE\"}")"
  printf '%s\n' "$response" > "$EVIDENCE/keyring_secret_set.json"
  echo "  $response"

  # The response is the detector. The measured bug answered `internal_error`
  # from a store that had already succeeded.
  case "$response" in
    *'"error"'*) fail "a successful store answered with an error envelope: $response" ;;
  esac
  case "$response" in
    *'"result"'*) ;;
    *) fail "the store did not answer with a result: $response" ;;
  esac

  # Where it landed, said by the engine rather than inferred by the client.
  # `store` is new in the fixed engine and optional on the wire, so an engine
  # that omits it is reported rather than refused. What must never pass is this
  # arm reading "file": that would mean a Secret Service was up and unlocked and
  # the value went somewhere else without anyone consenting to it.
  case "$response" in
    *'"store":"keyring"'*) echo "  the engine says it stored in the keyring" ;;
    *'"store":"file"'*)
      fail "an unlocked keyring was up and the value went to the file store: $response"
      ;;
    *) echo "  this engine publishes no store field; the keyring itself is checked below" ;;
  esac

  failures="$(journal_route_failures)"
  [ "$failures" = "0" ] ||
    fail "the store answered well but the journal records $failures route failure(s)"

  after="$(daemon_identity)"
  [ "$before" = "$after" ] ||
    fail "the daemon changed across a successful store: $before -> $after"

  # And the value is actually there. Without this the arm proves the engine
  # answered nicely, not that anything was stored.
  found="$(as_test secret-tool search --all service fermix 2>&1 | tr -d '\r')"
  printf '%s\n' "$found" > "$EVIDENCE/keyring_contents.txt"
  case "$found" in
    *"$SECRET_VALUE"*) echo "  the value is in the keyring, and the daemon is unchanged ($after)" ;;
    *) fail "the store reported success and the keyring does not hold the value" ;;
  esac

  # The shim's positive control. A successful store MUST have run the helper, so
  # a log with no store line here means the shim never intercepted -- and then
  # the locked arm's silence would prove nothing at all.
  local stores
  stores="$(shim_store_lines)"
  as_test cat "$SHIM_LOG" > "$EVIDENCE/keyring_shim_log.txt" 2>/dev/null
  [ "$stores" != "0" ] ||
    fail "the shim recorded no store at all on a successful save, so it is not on the engine's PATH"
  echo "  the shim intercepted $stores store invocation(s), so it is on the path"
  state_secrets_row keyring ready ready

  file_store_consent_round_trip
}

# The file store, entered by consent and left by the verb. This runs on the
# unlocked machine on purpose: it is the only arm where the way home can
# actually be taken, because migrating needs a keyring to migrate into.
#
# `store:"file"` in the request IS the consent -- per call, carried on the wire,
# never inferred from a failure. An engine that fell back to a file because the
# keyring was locked would be storing the owner's key somewhere they never
# agreed to, which is why the locked arm asserts nothing was written at all.
file_store_consent_round_trip() {
  step "the file store: entered by consent"
  local response dir_mode file_mode count
  response="$(call secret.set \
    "{\"id\":\"$FILE_SECRET_ID\",\"value\":\"$SECRET_VALUE\",\"store\":\"file\"}")"
  printf '%s\n' "$response" > "$EVIDENCE/consent_secret_set.json"
  echo "  $response"

  case "$response" in
    *'"store":"file"'*) echo "  the engine says the value landed in the file store" ;;
    *'"error"'*) fail "a consented file-store save was refused: $response" ;;
    *) fail "a store:\"file\" save did not report where it landed: $response" ;;
  esac
  state_secrets_row file ready consent

  # A secret in a world-readable file is not a store, it is a leak with a path.
  dir_mode="$(as_test stat -c %a "$HOME_PATH/secrets" 2>&1 | tr -d '\r')"
  [ "$dir_mode" = "700" ] ||
    fail "the secrets directory should be 0700 and is $dir_mode"
  file_mode="$(as_test sh -c "stat -c %a '$HOME_PATH/secrets/'* 2>/dev/null | sort -u" | tr -d '\r')"
  [ "$file_mode" = "600" ] ||
    fail "every file in the secrets directory should be 0600 and the modes are: $file_mode"
  count="$(secret_file_count)"
  echo "  $count file(s), 0600, in a 0700 directory"

  migration_home
}

secret_file_count() {
  as_test sh -c "ls -A '$HOME_PATH/secrets' 2>/dev/null | wc -l" | tr -d '\r'
}

# And out again. Without this verb the file store is a one-way door: a keyring
# write migrates a value back only when the owner SAVES it, and saving means
# retyping a key no client can read out of the file store. An owner who unlocks
# their keyring an hour later would have no way home at all.
migration_home() {
  step "the file store: and the way back out"
  local moved count
  moved="$(call secret.migrate_to_keyring '{}')"
  printf '%s\n' "$moved" > "$EVIDENCE/migrate.json"
  echo "  $moved"

  case "$moved" in
    *'"error"'*) fail "the migration back to the keyring was refused: $moved" ;;
  esac
  # The ids in `moved` are the ids `secret.set` TAKES, and the old spelling has
  # to fail here rather than merely not match. An engine returned the keyring's
  # env spelling -- TELEGRAM_BOT_TOKEN for telegram_bot_token -- and team-lead
  # ruled against it on 2026-09-20: a client that asks in one vocabulary and is
  # answered in another needs a translation table to know what moved, and a
  # table like that rots without anyone noticing. Refused by name, so the
  # failure says what is actually wrong rather than "did not report moving".
  case "$moved" in
    *"$(printf '%s' "$FILE_SECRET_ID" | tr '[:lower:]' '[:upper:]')"*)
      fail "the migration reports moved ids in the keyring's env spelling (TELEGRAM_BOT_TOKEN), not the spelling secret.set takes: $moved"
      ;;
  esac
  case "$moved" in
    *"\"$FILE_SECRET_ID\""*) ;;
    *) fail "the migration did not report moving $FILE_SECRET_ID: $moved" ;;
  esac
  case "$moved" in
    *'"store":"keyring"'*) ;;
    *) fail "the migration did not report ending in the keyring: $moved" ;;
  esac

  # Moved, not copied. A migration that leaves the value in both places has
  # left it in the one the owner believes they left.
  count="$(secret_file_count)"
  [ "$count" = "0" ] ||
    fail "the value migrated and $count file(s) remain in the file store"
  echo "  the file copy is gone"

  # And the row is back where it started, which is what makes the door two-way.
  state_secrets_row keyring ready migrated
}

# One arm, one machine, from a clean container to a verdict. The container is
# removed before the next arm starts, so neither arm can be standing in a state
# the other left.
run_arm() {
  local arm="$1" keyring="$2"
  start_machine "$keyring"
  # Recommendations follow the keyring: the arm that wants a Secret Service
  # takes the one the package recommends, and the arm that wants none refuses it.
  install_package "$keyring"
  engine_in_this_package
  # The KEYRING=0 image has no python3, because the install matrix uses that
  # image and should not gain packages for this row's sake. The management
  # socket still needs a client, so this arm installs one into the container
  # rather than into the image.
  [ "$keyring" = "1" ] || inside apt-get install -y -qq python3 ||
    fail "python3 could not be installed, and the management socket needs a client"
  start_engine
  [ "$keyring" = "1" ] && install_shim
  write_wire_client
  "$arm"
  cleanup
}

main() {
  parse_arguments "$@"

  EVIDENCE="$ROOT_DIR/packaging/out/smoke/keyring-$(printf '%s' "$IMAGE" | tr ':/' '--')"
  WORK="$(mktemp -d "${TMPDIR:-/tmp}/keyring-smoke.XXXXXX")"
  trap 'cleanup; rm -rf -- "$WORK"' EXIT
  rm -rf -- "$EVIDENCE"
  mkdir -p "$EVIDENCE"

  if wanted none; then run_arm arm_without_a_secret_service 0; fi
  if wanted keyring; then run_arm arm_with_an_unlocked_keyring 1; fi
  if wanted locked; then run_arm arm_with_a_locked_keyring 1; fi
  if wanted empty; then run_arm arm_with_no_default_collection 1; fi

  step "what ran"
  echo "  $IMAGE, arm(s) '$ARM', evidence in $EVIDENCE"
  echo
  echo "keyring_smoke: what did not run"
  echo "  the dialog, and everything that needs a person to read one. A"
  echo "  container can put the engine in all four states and check what it"
  echo "  answers; it cannot see what the window says about that answer, or"
  echo "  whether an unlock prompt appeared. Acceptance rows 16 to 19 are that"
  echo "  half and stay human."
  echo
  echo "keyring_smoke: ok"
}

main "$@"

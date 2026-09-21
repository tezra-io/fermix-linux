#!/bin/bash
# The harness for `keyring_smoke.sh`, offline: every refusal fires, every
# assertion the row exists to make is still in the script, and every wait is
# bounded. It starts no container -- the row itself needs Docker and a package,
# and a gate that cannot be run without both is a gate that is run rarely.
set -uo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT="$ROOT_DIR/scripts/keyring_smoke.sh"
DOCKERFILE="$ROOT_DIR/packaging/docker/Dockerfile.smoke"

failures=0

fail() {
  echo "keyring_smoke_test: $1" >&2
  failures=$((failures + 1))
}

refuses() {
  local why="$1"
  shift
  local output status
  output="$("$SCRIPT" "$@" 2>&1)"
  status=$?
  if [ "$status" -eq 0 ]; then
    fail "accepted $why"
    return
  fi
  case "$output" in
    *keyring_smoke:*) echo "  refused: $why" ;;
    *) fail "refused $why without saying so: $output" ;;
  esac
}

[ -x "$SCRIPT" ] || fail "keyring_smoke.sh is not executable"
bash -n "$SCRIPT" || fail "keyring_smoke.sh does not parse"

echo "keyring_smoke_test: what it refuses"
refuses "no package at all" --image ubuntu:24.04
refuses "a package that does not exist" /nonexistent/fermix-desktop.deb
refuses "an unknown option" --sideways "$ROOT_DIR/scripts/keyring_smoke.sh"
refuses "an image it has no package manager for" --image alpine:3.20 /etc/hostname
refuses "an arm it does not have" --arm sideways /etc/hostname
refuses "an rpm, which this row does not cover" --image ubuntu:24.04 /tmp/x.rpm
refuses "an expected engine tree with no digest after it" /etc/hostname --expect-tree

# The two arms are the row. A script that quietly ran one of them would still
# exit zero and would still look like a pass.
echo "keyring_smoke_test: the four arms"
for arm in \
  'arm_without_a_secret_service' \
  'arm_with_an_unlocked_keyring' \
  'arm_with_a_locked_keyring' \
  'arm_with_no_default_collection'; do
  grep -q "^$arm()" "$SCRIPT" || fail "the row no longer has $arm"
done
echo "  ok: all four arms are present"

# Four machines, and the point is that they answer differently. An engine that
# collapsed any two of them would be the defect this work exists to remove: a
# locked keyring reporting an absence is what the owner met on their own
# desktop, and it is why they could not save a key.
echo "keyring_smoke_test: the four answers stay four"
for answer in \
  'an absent store reported itself locked' \
  'a locked collection reported itself unavailable' \
  'a service with no collection reported itself locked'; do
  grep -qF -- "$answer" "$SCRIPT" ||
    fail "the row no longer refuses when two machines give one answer: $answer"
done
echo "  ok: absent, locked and collection-less cannot borrow each other's answer"

# A refusal must leave nothing behind, in either store. A refusal that stored
# the value anyway has already happened here on the unlocked path, so on the
# locked path it is checked rather than assumed.
grep -q 'nothing was stored' "$SCRIPT" ||
  fail "the locked arm does not check that a refused store stored nothing"
echo "  ok: a refusal on the locked path leaves both stores empty"

# The shim. A locked collection refused from the measured `Locked` property
# never runs the helper at all; one refused because the helper happened to exit
# usefully does. The two are indistinguishable from the envelope, so a logging
# `secret-tool` earlier on the PATH is what separates the fix from a
# coincidence -- verified 2026-09-20 that the unit's PATH on ubuntu ranks
# /usr/local/bin second and /usr/bin fourth.
echo "keyring_smoke_test: the shim that tells a fix from a coincidence"
grep -q '/usr/local/bin/secret-tool' "$SCRIPT" ||
  fail "the row installs no secret-tool shim, so it cannot see whether the helper ran"
grep -q 'exec /usr/bin/secret-tool' "$SCRIPT" ||
  fail "the shim does not exec the real secret-tool, so it would break the arm it instruments"

# The positive control, which is what makes the negative one mean anything: in
# the UNLOCKED arm the log must contain a store line. Without it, "no store
# line" on the locked path could just mean the shim was never on the path.
grep -q 'the shim recorded no store at all' "$SCRIPT" ||
  fail "the unlocked arm does not prove the shim intercepts"
echo "  ok: the shim logs and delegates, and the unlocked arm proves it intercepts"

# Whether the locked arm REFUSES on a store line or merely records one is a
# question about what the engine promises, not about this gate, so it is a
# named switch rather than a silent choice.
grep -q '^SHIM_VERDICT=' "$SCRIPT" ||
  fail "the locked arm's use of the shim log is not a declared choice"
echo "  ok: asserting on the shim log is a declared choice"

# The assertions, one by one. Every one of these was learned from a machine
# rather than reasoned about, and a script that stopped making one would go on
# passing.
echo "keyring_smoke_test: what each arm asserts"

# The absent-store arm: the reason must be `unavailable` and never `locked`,
# because absent and locked are different machines with different remedies and
# one message for both is the thing this work removes.
for assertion in \
  'secret_store_failed' \
  '"reason":"unavailable"' \
  'org.freedesktop.secrets'; do
  grep -qF -- "$assertion" "$SCRIPT" ||
    fail "the absent-store arm no longer asserts $assertion"
done

# The unlocked arm, and this is the list that matters. A successful keyring
# write returned `{:ok, ""}` to a write log with no clause for it, so the store
# SUCCEEDED, the route raised, and the caller was told the save failed -- while
# the value sat in the keyring. Measured on 2026-09-20 against the package
# e6d4b266. Three consequences, all of them assertions here:
#
#   the daemon does NOT die, so pid and restart_count alone pass a broken
#   machine and are necessary rather than sufficient;
#   the response is the detector -- a result, not an error envelope;
#   and the journal names the failure even when the client does not.
for assertion in \
  'Management route failed' \
  '"error"' \
  'restart_count' \
  'secret-tool' \
  '"store":"file"' \
  'Locked'; do
  grep -qF -- "$assertion" "$SCRIPT" ||
    fail "the unlocked arm no longer asserts $assertion"
done
echo "  ok: the reason, the response, the journal, the daemon and the value"

# The value must be read back out of the keyring. Without it the arm proves the
# engine answered well, not that anything was stored.
grep -q 'secret-tool lookup\|secret-tool search' "$SCRIPT" ||
  fail "the unlocked arm never reads the value back out of the keyring"
echo "  ok: the stored value is read back"

# The state the APP reads. Every assertion above this line is about what
# `secret.set` answers, and the app does not draw that answer -- it draws the
# Settings row, which comes from `setup.state.get`. The two can disagree: the
# store field moved out of the top level into a nested `secrets` row, and an app
# build reading the old shape drew nothing while the engine answered perfectly.
# Nothing in the container lane could see that, which is the gap this closes.
# Narrowly: the row's SHAPE and VALUE on the wire, not what any window does with
# it. Row 18 still reads the window by eye.
echo "keyring_smoke_test: the row the Settings pane reads"
grep -q 'setup.state.get' "$SCRIPT" ||
  fail "no arm asks for the state the Settings row is drawn from"
grep -qF -- '"secrets"' "$SCRIPT" ||
  fail "the row does not assert the nested secrets object, which is where the field moved to"

# Four machines, four rows, and they are not the same four words as the
# refusal reasons: `store` says where this home saves (always keyring until
# someone consents to a file), `availability` says whether it could save right
# now. A single field for both is the conflation this work removes.
for pair in \
  'state_secrets_row keyring unavailable' \
  'state_secrets_row keyring ready' \
  'state_secrets_row keyring locked'; do
  grep -qF -- "$pair" "$SCRIPT" ||
    fail "no arm asserts the Settings row reads: $pair"
done
echo "  ok: absent, ready and locked each have a declared Settings row"

# The consent round trip, which is the only path that writes the file store at
# all. It has to end where it started or the owner is stranded there: a file
# store nobody can leave is a worse place than a locked keyring.
echo "keyring_smoke_test: the file-store consent round trip"
for assertion in \
  '"store":"file"' \
  'secret.migrate_to_keyring' \
  'state_secrets_row file ready'; do
  grep -qF -- "$assertion" "$SCRIPT" ||
    fail "the consent step no longer asserts $assertion"
done
grep -q 'consent' "$SCRIPT" ||
  fail "the row has no file-store consent step"

# The modes are the point of a file store: a secret in a world-readable file is
# not a store, it is a leak with a path.
grep -qF -- '0600' "$SCRIPT" ||
  fail "the consent step does not check the secret file's mode"
grep -qF -- '0700' "$SCRIPT" ||
  fail "the consent step does not check the secrets directory's mode"

# And the way home has to actually empty the file store. A migration that
# copies without deleting leaves the value in two places, one of which the
# owner thinks they left.
grep -q 'the file copy is gone' "$SCRIPT" ||
  fail "the migration is not checked to have removed the file copy"

# The ids in `moved` are the ids `secret.set` TAKES -- telegram_bot_token, not
# the keyring's env spelling TELEGRAM_BOT_TOKEN. Ruled on 2026-09-20 after an
# engine returned the second. A client that asks in one vocabulary and is
# answered in another has to keep a translation table to know what moved, and a
# table like that rots silently. The old spelling must FAIL this row, so it is
# refused by name rather than merely not matched.
grep -qF -- 'TELEGRAM_BOT_TOKEN' "$SCRIPT" ||
  fail "the migration does not refuse the env spelling of the moved ids by name"
echo "  ok: consent writes a 0600 file in a 0700 dir, and the verb brings it home"

# And no arm should be spent discovering that the package carries the wrong
# engine. The tree digest is inside the package, so it can be read before the
# first arm runs: build_id cannot separate two engines and the tree digest can.
echo "keyring_smoke_test: which engine is in this package, before any arm runs"
grep -q 'engine_tree_sha256' "$SCRIPT" ||
  fail "the row never reads which engine the package carries"
grep -q -- '--expect-tree' "$SCRIPT" ||
  fail "the row cannot be told which engine tree it is meant to be testing"
echo "  ok: the engine in the package is named before the arms spend a container"

echo "keyring_smoke_test: the container it declares"
grep -q 'ARG KEYRING=0' "$DOCKERFILE" ||
  fail "the smoke container has no KEYRING argument, so this row has no Secret Service"
grep -q 'gnome-keyring' "$DOCKERFILE" ||
  fail "the smoke container installs no Secret Service implementation"
grep -q 'python3' "$DOCKERFILE" ||
  fail "the smoke container has no python3, and the management socket needs a client"
grep -q 'KEYRING=\$keyring' "$SCRIPT" ||
  fail "the row does not pass a keyring build argument to the image"

# Two arms, two machines, and which machine each arm gets is the thing that was
# wrong the first time this ran. With gnome-keyring INSTALLED,
# org.freedesktop.secrets is a D-Bus activatable name whether or not a daemon is
# running, so the absent case cannot be shown on that image at all. The absent
# arm must therefore get a machine where nothing implements the interface.
grep -q 'run_arm arm_without_a_secret_service 0' "$SCRIPT" ||
  fail "the absent-store arm no longer runs on a machine without a Secret Service"

# And the package RECOMMENDS gnome-keyring, which apt installs by default, so
# the absent arm must refuse recommendations or it installs the very thing it
# is trying to do without.
grep -q -- '--no-install-recommends' "$SCRIPT" ||
  fail "the absent-store arm takes the recommended keyring, so it proves nothing"
grep -q 'run_arm arm_with_an_unlocked_keyring 1' "$SCRIPT" ||
  fail "the unlocked arm no longer runs on a machine with a keyring installed"
echo "  ok: one image argument, and an arm apiece"

# The install matrix must NOT gain a keyring: its rows prove the absent-store
# path, and a keyring daemon in every image would change what they prove
# without changing what they say.
grep -q 'KEYRING=1' "$ROOT_DIR/scripts/install_smoke.sh" &&
  fail "the install smoke now builds a keyring image, which changes what its rows prove"
echo "  ok: the install matrix still runs without a Secret Service"

echo "keyring_smoke_test: every wait is bounded"
for cap in SYSTEMD_ATTEMPTS USER_MANAGER_ATTEMPTS KEYRING_ATTEMPTS; do
  grep -q "^${cap}=[0-9]" "$SCRIPT" || fail "$cap is not a declared number"
done
if grep -qE 'while (true|:)' "$SCRIPT"; then
  fail "the row contains an unbounded loop"
fi
echo "  ok: three declared caps, and no unbounded loop"

if [ "$failures" -ne 0 ]; then
  echo "keyring_smoke_test: $failures check(s) failed" >&2
  exit 1
fi
echo "keyring_smoke_test: every refusal fired"

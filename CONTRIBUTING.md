# Contributing

Two things in this repository are easy to get wrong quietly, so both are written
out here rather than left to memory: the contract pairing rules, and the
re-vendor recipe.

## The contract pairing rules

Two contracts are authored in the engine, published by it, and vendored here:
the daemon's management protocol at `apps/fermix_core/priv/management/`, which
this application reads over `daemon.sock`, and the typed `--json` verbs of the
packaged command line at `apps/fermix_core/priv/cli/`, which are the only thing
that answers while the daemon is stopped. This repository holds one verbatim
copy of each, under `App/Fermix/contracts/`, pinned by two records that have to
agree. Every rule below applies to both; `SOURCE.json` lists them, and listing a
contract there is what puts it under the pin.

- **Never edit a vendored file by hand.** Not to make a client compile, not to
  add a field the design says is coming. `tests/contract.rs` recomputes every
  digest at test time, so an edit here is a red build rather than a silent
  divergence from the engine.
- **Start from a committed engine commit.** A tree taken from an uncommitted
  working copy has no upstream anybody else can compare against.
  `SOURCE.json`'s `committed_upstream` says which you took, and
  `scripts/verify_contract.sh` prints a note when it is false.
- **The engine ships first.** A wire change lands in the engine, is released,
  and is re-vendored here afterwards. Never ship a client that requires a
  version the released daemon lacks; roll a client back to `N` before dropping
  `N` from the daemon.
- **The version window comes from the schema**, through
  `x-supported-version-range`, and there is exactly one such key.
  `tests/contract.rs` asserts the schema publishes no second one, because a
  second key drifts from the first and the drift is invisible until a
  negotiation fails in the field.
- **A field a newer daemon adds to an existing method binds optionally, with a
  declared absent rendering.** The direction that breaks is a new client talking
  to an old daemon. No result struct in `management/types.rs` denies an unknown
  field; every additive one carries `#[serde(default)]` and a comment saying
  what absent renders as.

## The re-vendor recipe

Five steps and a verification. `<fermix>` is a checkout of the engine at the
commit you are pinning.

```sh
FERMIX=../fermix                       # a checkout of tezra-io/fermix
DST=App/Fermix/contracts

# 1. Copy the published trees, verbatim. Re-vendor the one that moved; the
#    other's bytes and its record stay exactly as they are.
rm -rf "$DST/management"
cp -R "$FERMIX/apps/fermix_core/priv/management" "$DST/management"
rm -rf "$DST/cli"
cp -R "$FERMIX/apps/fermix_core/priv/cli" "$DST/cli"

# 2. Regenerate the digests, over an LC_ALL=C sorted file list so the ordering
#    is reproducible.
( cd "$DST" && find . -type f ! -name CHECKSUMS.txt ! -name SOURCE.json |
    sed 's|^\./||' | LC_ALL=C sort | xargs shasum -a 256 > CHECKSUMS.txt )

# 3. Edit SOURCE.json by hand, in the entry for the contract you re-vendored:
#    the upstream commit, the branch, the working-tree state at retrieval, the
#    retrieval date, every sha256, the version facts that entry carries,
#    committed_upstream, and a provenance note saying what moved and why the
#    window did or did not.
$EDITOR "$DST/SOURCE.json"

# 4. Verify, including against upstream itself.
scripts/verify_contract.sh --source "$FERMIX"
cd App/Fermix && cargo test --test contract

# 5. Re-take every capture, because many of them are drawn from the goldens you
#    just replaced. No arguments: all of them, and each one's INDEX.md row is
#    rewritten with it.
scripts/capture.sh --container
```

Step 4 is the one that matters. `verify_contract.sh` alone proves the two
records agree with the bytes; `--source` is the only check that can see upstream
having moved ahead of the vendored copy, and it is what a re-vendor must be
verified with.

If `cargo test --test contract` then fails, the engine changed a shape this
client decodes. Fix the client, never the vendored file.

Step 5 is not tidying. Many captures answer `setup.state.get` straight from the
vendored fixtures — 29 of the 46 capture references, measured on 2026-09-20 —
and the panes they draw hide a row entirely when its field is absent rather
than drawing it wrong. So a capture taken after a field moves shows a group
missing, which a reviewer sees at once, and a capture taken before it moved
shows the old row perfectly and proves nothing while still looking like
evidence that somebody checked.

Re-take all of them, and never a chosen subset. Which captures depend on the
golden changes with the method that moved, so any list of "the affected ones"
is right on the day it is written and silently wrong afterwards — and being
silently wrong here produces exactly the stale capture the step exists to
prevent. `scripts/capture.sh --container` with no further arguments takes every
capture and rewrites each one's row in `docs/design/captures/INDEX.md`.

Re-taking also sends the reviewer and decision columns of every capture it
touches back to `pending`, which is the point rather than a side effect: a new
image has not been looked at, and the index should not claim otherwise. Expect
the review pass that follows a re-vendor to be a real one. Nothing enforces
this step yet, so it is on you.

One more audience runs over the same records: `scripts/verify_contract.sh
--release`, which the release rail runs and which turns the `committed_upstream`
note into a refusal. A contract vendored from an engine working tree pins bytes
nobody else can retrieve, and that is a developer's convenience rather than
something a release may carry.

## The gates

Every gate has a `*_test.sh` beside it that drives its refusals against a
throwaway copy. If you add a gate, add its harness in the same change: a gate
nobody has seen fail is a gate nobody knows works.

```sh
scripts/verify_contract.sh          scripts/verify_contract_test.sh
scripts/check_app_identity.sh       scripts/check_app_identity_test.sh
scripts/check_copy.sh               scripts/check_copy_test.sh
scripts/check_vendor_marks.sh       scripts/check_vendor_marks_test.sh
scripts/check_no_network.sh         scripts/check_no_network_test.sh
scripts/check_copyright.sh          scripts/check_copyright_test.sh
scripts/check_private_runtime.sh    scripts/check_private_runtime_test.sh
scripts/container_build.sh          scripts/container_build_test.sh
scripts/build_packages.sh           scripts/build_packages_test.sh
scripts/verify_engine.sh            scripts/verify_engine_test.sh
scripts/engine_bump.py              scripts/engine_bump_test.sh
scripts/asset_name.sh               scripts/asset_name_test.sh
scripts/runtime_watch.py            scripts/runtime_watch_test.sh
scripts/install_smoke.sh            scripts/install_smoke_test.sh
scripts/render_install_notes.sh     scripts/render_install_notes_test.sh
packaging/runtime/build_runtime.sh  packaging/runtime/build_runtime_test.sh
```

Two scripts have no harness of their own, deliberately. `scripts/render_icons.sh`
generates the packaged icon set, and what it wrote is gated twice already:
`tests/packaging.rs` holds every raster to its directory's size and holds the
scalable and symbolic icons byte-identical to the application's own, and
`scripts/check_app_identity.sh` holds every file name to the application
identity. `scripts/capture.sh` renders the reference captures, and a reviewer
looking at them is the gate.

## Changing something the package installs

`packaging/` is what the `fermix-desktop` package puts on a machine, and every
way one of those files can be wrong is silent: a launcher that does not appear,
a window that does not group under its icon, an application invisible in the
software centres, a second launch that opens a second window. So a change there
is a change in four places at once, and the gates hold them together:

- a file added or removed joins `INSTALLED_PATHS` in `App/Fermix/tests/packaging.rs`
  and the `contents:` list in `packaging/nfpm-fermix-desktop.yaml.tmpl`, or the
  suite fails;
- a new host dependency is declared in both families in that same template, and
  `scripts/package_dependencies.py` then checks that nothing in the package
  links outside `/usr/lib/fermix-desktop` except to a declared host relation;
- a new bundled library joins `packaging/runtime/RUNTIME.lock.json` with its
  licence, or `scripts/check_copyright.sh` fails, and the runtime image key
  changes so `runtime.yml` rebuilds and republishes the image before any package
  build can pull it;
- the application identity goes in every place `scripts/check_app_identity.sh`
  lists, which is now all six of them;
- a change to `App/Fermix/resources/icons/` is followed by
  `scripts/render_icons.sh --container` and a commit of what it wrote.

`docs/RELEASING.md` is the procedure for getting any of it to a person.

The container build is the run that matters: the development machine is usually
not a Linux desktop, and a build that has only ever happened against Homebrew's
GTK has proven nothing about the toolkit floor.

## Before you open a pull request

```sh
cd App/Fermix
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cd ../..
scripts/container_build.sh
```

`cargo test` on its own does **not** run the widget suite: it needs a display
and runs only with `FERMIX_GTK_TESTS=1`, which `scripts/container_build.sh` and
`scripts/build_packages.sh` set for you. A skipped widget run and a passing one
both print `1 passed`, and the only thing separating them is the duration —
`finished in 0.00s` is a skip, not a pass. To be certain it executed, run it
with `-- --nocapture` and confirm the `skipped:` line is absent.

If the change touches `packaging/`, `engine/PIN.json` or anything under
`scripts/`, build the package too, which runs every gate above inside the
container, and install it on the floor:

```sh
export DOCKER_HOST=unix:///var/run/docker.sock

FERMIX_DESKTOP_BUILD_ID=local-1 \
FERMIX_DESKTOP_SOURCE_COMMIT=$(git rev-parse HEAD) \
  scripts/build_packages.sh <version> <amd64|arm64> --container

scripts/install_smoke.sh --image ubuntu:22.04 packaging/out/packages/fermix-desktop_<version>_amd64.deb
```

A tree with no pinned engine is not buildable, because the package contains the
engine. On a machine with no Elixir, build one from the engine checkout and pass
it: `docs/RELEASING.md` has the two commands.

Say which gates are not applicable and why. Never mark work done without proof.

# Releasing Fermix for Linux

One package reaches a Linux desktop. `fermix-desktop` carries the window, the
engine and the private GTK4 runtime the window draws with, so a person runs one
`apt install` or one `dnf install` and has the whole product. The engine-only
`fermix` package still exists for headless hosts, is built and released in
`tezra-io/fermix`, and is an alternative rather than a layer: each package
provides, conflicts with and replaces the other.

Nobody edits a pin and nobody types a version. An engine release drives a
desktop release, and a person is involved twice: at the engine tag, and at the
protected `release-linux` environment when the packages are ready to publish.

## The chain, from an engine tag to a release page

1. A person pushes `vX.Y.Z` to `tezra-io/fermix`.
2. That repository's `release.yml` runs as it does today and publishes, among
   the rest, `fermix_app_engine_linux_x86_64.tar.gz` and
   `fermix_app_engine_linux_aarch64.tar.gz`, cosign-signed, each with its
   `.sha256`, `.sig` and `.pem`.
3. Its `dispatch-linux-desktop` job, which runs after `promote` so every asset
   is certainly downloadable, reads both `.sha256` sidecars off the published
   release and sends a `repository_dispatch` here of type `engine-released`,
   carrying the tag, the version, the source commit, the certificate identity
   that tag signs as, and each archive's name and digest.
4. `engine-release.yml` receives it. `scripts/engine_bump.py` validates every
   field of the payload against what this repository derives itself, writes
   `engine/PIN.json`, `App/Fermix/Cargo.toml`, `App/Fermix/Cargo.lock` and the
   metainfo's release entry, and then the workflow downloads and verifies the
   archives the pin names **before the commit is made**. It pushes
   `engine/vX.Y.Z` and opens a pull request with auto-merge enabled.
5. The pull request's checks are `app.yml`, `contract.yml` and `packages.yml`,
   which include a real package build and an install smoke on the floor. Nothing
   merges on a red check, and a failed check leaves an open pull request with the
   failure on it, which is the retry surface.
6. When that lands on `main`, the `tag` job in the same workflow pushes
   `fermix-desktop-vX.Y.Z`. It refuses to tag anything but the bot's own bump
   line, refuses a version the crate disagrees with, and refuses a version that
   is already published.
7. `release-fermix-desktop.yml` runs on that tag.
8. Its `publish` job waits in the protected `release-linux` environment for its
   required reviewer.

### Why the bot cannot use `GITHUB_TOKEN`, and what that costs

GitHub starts no workflow run from an event the default token produced. A pull
request opened with `GITHUB_TOKEN` fires no `pull_request` workflows, so
auto-merge would have nothing to wait on; a tag pushed with it fires no `push`
tag workflows, so the release would never run. Both are silent: the chain stops
with everything green and nothing published.

So steps 3, 4 and 6 mint a GitHub App installation token with
`actions/create-github-app-token`, pinned by commit SHA like every other action.
The cost is that the App's credentials are secrets in both repositories. The
property that matters is preserved by branch protection rather than by token
scope: `main` requires the full check set, the App holds no Administration
permission and cannot relax it, so the worst an actor holding the App's key can
do is open a pull request and wait for the same gates as everybody else, and
then be stopped again at the `release-linux` environment.

### What the owner sets up by hand, once

| Where | What |
|---|---|
| A GitHub App, `Fermix Release Bot` | Installed on `tezra-io/fermix-linux` only. Repository permissions: **Contents: write**, **Pull requests: write**. Nothing else: no Actions, no Administration, no organisation permission. Contents write is what `repository_dispatch` requires |
| `tezra-io/fermix`, repository secrets | `FERMIX_RELEASE_APP_ID`, `FERMIX_RELEASE_APP_PRIVATE_KEY` |
| `tezra-io/fermix-linux`, repository secrets | The same two, for the same App |
| `tezra-io/fermix-linux`, settings | Allow auto-merge. Branch protection on `main` requiring `app`, `contract` and `packages`, with no bypass |
| `tezra-io/fermix-linux`, environments | `release-linux`, with a required reviewer |
| `tezra-io/fermix-linux`, packages | `ghcr.io/tezra-io/fermix-desktop-runtime` readable by this repository's workflows |

One pin is on the credential path and is worth a sentence.
`actions/create-github-app-token` is pinned by commit SHA like every other
action, and `App/Fermix/tests/packaging.rs` proves every `uses:` is an immutable
40-hex commit carrying a version comment. What no gate can prove is that the
commit is the one the tag pointed at. So it was confirmed by a second
independent resolution rather than by review: the SHA was resolved from the tag
through the GitHub API on 2026-09-19, and the coordinating session resolved the
same tag separately on the same day and got the same commit,
`bcd2ba49218906704ab6c1aa796996da409d3eb1`, with the moving `v3` tag resolving
to it as well. Both workflows that use the action record that beside the pin.

**Re-confirm it at the first real release**, and add who did so and when to
those comments. Not because it is unverified, but because it is the one fact a
later reader cannot recover from the files, and because a credential-path pin is
the one place where a stale confirmation is worth less than a fresh one.

## The version rule

`fermix-desktop 0.11.0` carries engine 0.11.0. The tag, `Cargo.toml`, the
metainfo and the pin must agree, and `scripts/build_packages.sh` and the release
rail both refuse otherwise.

**A desktop-only fix is `<engine version>+<n>`.** `0.11.0+1` for the first,
`0.11.0+2` for the second: a toolkit CVE rebuild, a copy fix, a packaging fix.
`+` lives in the upstream version in both families, so nFPM's single `release`
field stays empty, the rpm keeps its `-1`, and both `dpkg --compare-versions`
and `rpmdev-vercmp` order `0.11.0 < 0.11.0+1 < 0.11.1`. A `+N` release carries
the same engine as the `X.Y.Z` it rebuilds, so the pin's `engine_version` is the
part before the `+`, and the release rail checks that.

The refusal on `-` and `:` stays. No Debian revision, no rpm epoch, and **a
prerelease tag produces no packages at all**.

### What `+` is proven to do, and the one row that is not

Six places handle the character. Five are settled, one is not.

| Where | State |
|---|---|
| Git tag ref | Settled. `+` is not in git's forbidden set |
| Cargo | Settled. `0.11.0+1` is valid semver build metadata, and cargo ignores build metadata when comparing versions. Nothing here compares crate versions and `build_packages.sh` compares the strings itself. Recorded so that nobody adds a comparison later |
| `dpkg --compare-versions` and librpm | **Proven.** `0.10.5 < 0.10.5+1 < 0.10.5+2 < 0.10.6` orders correctly in both |
| nFPM output file names | **Proven** against real artifacts: nFPM 2.47, with `version_schema: none` and an empty `release`, keeps the `+` verbatim in both version fields and both file names. A `0.10.5+1` build produced `fermix-desktop_0.10.5+1_amd64.deb` and `fermix-desktop-0.10.5+1-1.x86_64.rpm` |
| AppStream | `appstreamcli validate` runs on every push over the metainfo, so a `<release version="0.11.0+1">` it rejected would fail in `app.yml` rather than at release time |
| **GitHub release asset names** | **Unproven.** GitHub rewrites some characters in an uploaded asset's name. If `+` becomes something else, the published name and the name `gh release download --pattern` asks for diverge |

That last row goes through one function, `scripts/asset_name.sh`, which every
place that writes or asks for an asset name calls. Its constant `ASSET_PLUS` is
`+` today. If the dry run shows GitHub rewriting the character, `ASSET_PLUS`
becomes `~plus~` in one edit and every caller changes with it; the version a
user sees and the installed package do not change at all, only the asset file
name. `scripts/asset_name_test.sh` covers both settings already, so that edit is
one line and no decision.

The rail does not wait for the dry run to notice. `release-fermix-desktop.yml`'s
`publish` job asks the release page for every asset back, by the name it
published it under, and compares the bytes. A `+` GitHub rewrote fails there, at
the end of the rail, rather than in front of a person following the install
instructions.

**The dry run**, which is what actually answers the two rows, and which cannot
be run from a development machine because it needs the organisation's GitHub:

1. On a branch, set the crate, the metainfo and the tag to `0.11.0+1` against a
   pin whose `engine_version` is `0.11.0`.
2. Push `fermix-desktop-v0.11.0+1`. If the tag itself is refused, the first row
   of the table is wrong and nothing else matters.
3. Read the `build` job's log for the file names nFPM chose, and confirm they
   are the ones the table above records.
4. Let `publish` run into the `release-linux` environment and approve it against
   a **draft** release: change the `gh release create` line to add `--draft` for
   the dry run only.
5. Read the asset names off the draft release page, and read the `Ask for every
   asset back by name` step. That answers the GitHub row, either way, and it is
   the only thing the dry run is still needed for.
6. Download the deb and the rpm by name and run
   `scripts/install_smoke.sh --image ubuntu:22.04 <deb>` and
   `scripts/install_smoke.sh --image almalinux:9 <rpm>` against them, then assert
   the ordering by hand:

   ```sh
   dpkg --compare-versions 0.11.0 lt 0.11.0+1 && echo "deb orders it"
   rpmdev-vercmp 0.11.0 0.11.0+1
   ```

7. Delete the draft release and the tag. Nothing was published, so nothing is
   re-cut.

Until step 5 has been run and its answer written into this document, **the
asset name of a `+N` release is the one thing about it that is not proven**.
Everything a package manager reads is. The rail refuses rather than publishes a
divergence, so the worst case is a failed release rather than an install
instruction that does not work.

## What the rail does

| Job | Does |
|---|---|
| `version` | Reads the version out of the tag and refuses anything either family would read differently, holds it equal to `Cargo.toml` and to the metainfo, refuses an unpinned engine, refuses a `+N` whose engine version is not the part before the `+`, computes the runtime image key, and refuses a tag that is already published |
| `runtime` | Asks `packaging/runtime/build_runtime.sh --print-key` for the key this lock file names, and checks that `ghcr.io/tezra-io/fermix-desktop-runtime:<key>-<arch>` is published for both architectures. It builds nothing: a runtime compile is about an hour per architecture and belongs to `runtime.yml`, so a release pulls rather than waits |
| `engine` | Downloads the two archives `engine/PIN.json` names and verifies each against the pin's digest and the pinned cosign identity. What travels to the builds is the download, not a staging tree: `build_packages.sh` verifies it again before it unpacks, and a tree handed to it would be a tree it did not verify itself |
| `build` | One runner per architecture, `ubuntu-24.04` and `ubuntu-24.04-arm`, each running the shared `build-packages` action, which runs `scripts/build_packages.sh` in the build container against the verified engine and the pulled runtime |
| `sign` | Assembles `fermix_desktop_runtime_sources_<version>.tar.gz` from the lock file, then `cosign sign-blob` keyless over the four packages and that tarball, a `.sha256` beside each, and a `verify-blob` of what it just signed |
| `verify` | The install smoke matrix below |
| `publish` | In the protected `release-linux` environment: renders `INSTALL.md`, names every asset through `scripts/asset_name.sh`, creates the release with `--latest`, and then asks for every asset back by name and compares the bytes |

`--latest` is true, where it used to be false. It was false while this page
carried half of a pair and the engine's own release page was the primary one.
One package per family means this page is the product, and the newest tag is the
one a person should download.

**A release note describes the package's own engine, not the newest one.** The
desktop package carries an engine inside it, so a fix that landed in the engine
repository is only in this release if this release's engine has it — read
`engine_tree_sha256` from `/usr/share/fermix-desktop/build.json` and say what
that build does, rather than what the branch does. The case that made this a
rule: the app gained dialogs for a locked keyring whose trigger is the engine
answering `locked`, and an engine that cannot measure a lock never answers
`locked` at all — it reports a `timeout`, because a locked collection makes the
helper block rather than exit. A note saying the desktop fixes the keyring
message would send someone to the very failure they already had, and they would
conclude the fix does not work. The honest sentence names the engine build the
fix arrives with.

Every action is pinned to an immutable commit SHA. There is no notarization, no
cask and no Sparkle here, and each absence is deliberate: none of the three has
a Linux referent. What remains is the keyless cosign signature over each
artifact, and, when the repository service exists, its own GPG signature over
`InRelease` and `repomd.xml.asc`, which is the signature a user's package
manager actually verifies.

## The install smoke matrix

```sh
scripts/install_smoke.sh --image ubuntu:22.04 packaging/out/packages/fermix-desktop_0.11.0_amd64.deb
scripts/install_smoke.sh --image fedora:44 --wayland packaging/out/packages/fermix-desktop-0.11.0-1.x86_64.rpm
```

Each row stands up a container running systemd as process one, installs the one
package through that family's own package manager, grants linger to an ordinary
account, runs `fermix service install --json --home "/home/test/fermix home"`
and `fermix service status --json` as that account, and then opens the window on
a display and photographs it. Evidence lands in
`packaging/out/smoke/<image>/`.

| Image | Family | Arch | What it proves |
|---|---|---|---|
| `ubuntu:22.04` | deb | amd64, arm64 | The oldest deb target. GTK 4.6 on the host, and the window still draws |
| `ubuntu:24.04` | deb | amd64, arm64 | |
| `ubuntu:26.04` | deb | amd64 | The newest deb target. Also under Wayland |
| `debian:12` | deb | amd64 | Debian oldstable, in LTS |
| `debian:13` | deb | amd64 | Debian stable |
| `almalinux:9` | rpm | amd64 | The oldest rpm target, and the glibc floor the build base sets |
| `fedora:44` | rpm | amd64 | The newest rpm target. Also under Wayland |

**These rows are the releases supported on 2026-09-19, and they are not the ones
the amendment's section 8.4 table names.** Fedora 42 was end of life before this
was written, and Ubuntu 25.10 reached end of life on 9 July 2026. Fedora supports
the current release and the one before it, which is 44 and 43; Ubuntu 26.04 LTS
was released on 23 April 2026 and is the newest release; Debian 13 trixie is
stable and Debian 12 bookworm is oldstable in LTS. So the newest row of each
family moved, Ubuntu 25.10 to Ubuntu 26.04 and Fedora 42 to Fedora 44,
and the two floors the amendment sets deliberately did not: `ubuntu:22.04` is
still in standard support into 2027 and `almalinux:9` is still the glibc
floor. `scripts/install_smoke_test.sh` reads the image names out of this
document and refuses any this gate has no package manager for, so the two cannot
drift apart.

**Pin the package by digest, not by path.** `--expect-package <sha256>` hashes
the file and refuses before starting a container. Use it on every row of a real
acceptance run, because the thing this catches is not a wrong path: it is a
correct path holding a different build. On 2026-09-20 two pairs were handed over
minutes apart, same version, same engine tree, same contract, same packaging,
differing only in application code — and every identity check in this gate
passed on the wrong one. `--expect-tree` cannot separate them, since the engine
tree is identical; only the package's own digest can.

**The upgrade row: `--upgrade-from <older package>`.** Every other row installs
onto a clean machine, and an owner does not have one. This row installs an older
package, starts its user service, installs the package under test over the top,
reads `service status` before any restart, restarts the user unit the way the
product does, and then asks what actually answers:

| Assertion | Why it is not the obvious one |
|---|---|
| `alignment` is `pending_restart` before the restart | reporting both halves from the identity compiled into whichever binary answered makes a stale machine report `aligned`, and then nothing tells the owner to act |
| `need_daemon_reload` is false after the restart | nothing running as root can reload another account's user manager, so without a reload the restart faithfully relaunches the superseded command line |
| the **running** build id equals `engine_build_id` in the installed `build.json` | the `installed` half of that same response can read old for a different reason |
| the new engine's module is in the extracted payload | the packaged wrapper is new on disk whether or not it ever unpacked anything |
| the daemon's own `hello` publishes the new verb | a module in the extraction is an input; the published list is what the application can call |

This row exists because an upgrade over a running engine was tested nowhere, and
an owner met the consequence: a new application talking to an engine from hours
earlier, permanently, because the extracted payload was keyed on product version
alone and so was never replaced. A restart produced a new process running the
same old code.

**Every probe that searches for something carries a positive control, and the
reason is worth keeping.** A search that cannot run reports the same clean zero
as a search that ran and found nothing: `strings` is absent from `debian:13`, a
compressed payload matches no string at all, and a moved path matches nothing by
definition. So each probe also looks for something it MUST find, and refuses
when that is missing rather than reporting the real needle as absent — and for
a near-miss that must NOT be found, so a hit is not an artefact of the matching.
Three separate checks in this gate were vacuous before that rule was applied,
and all three looked green.

**Two displays, not one.** Every row draws the window under Xvfb, which
exercises the X11 backend. The newest image of each family draws it a second
time under a headless Weston with `GDK_BACKEND=wayland`, and photographs that
too. Xvfb proves nothing about the private `libwayland-client`, which is the
component this design replaces at a version the host does not have and the
component whose failure mode is a window that never appears. The older images
keep the X11 run only, because a current Weston needs a backport there, and the
gap is named rather than closed: Ubuntu 22.04's Wayland path is covered by the
acceptance runbook's real-session rows.

**The official images are minimised in three ways that break this gate for
reasons belonging to the image, and `Dockerfile.smoke` undoes all three.** Ubuntu
strips `/usr/share/doc` through `dpkg.cfg.d/excludes` and Fedora through
`tsflags=nodocs`, while `dpkg -L` and `rpm -ql` go on listing the file that was
thrown away, so the runtime manifest looks missing on half the matrix from one
package. AlmaLinux masks `systemd-logind.service` by symlinking it to
`/dev/null`, so `loginctl enable-linger` fails and there is no user manager to
install a user unit into. Both were found by running the matrix, both read as
product bugs, and neither is one. If you add a row, expect a third such thing and
look at the image before you look at the package.

The documentation exclusion has a consequence beyond the container, and the gate
is built around it: a person can set that exclusion on a real machine, so
anything under `/usr/share/doc` is advisory and nothing load-bearing may live
there. The runtime manifest is therefore checked **in the package**, with
`dpkg-deb -c` and `rpm -qlp`, where it is guaranteed; the installed tree is asked
instead for `identity.json` under the private prefix, which no exclusion reaches.
A gate demanding the manifest on disk would promise something the package
manager does not.

Weston needs two flags that are not obvious either: `--renderer=pixman`, because
the headless backend composites nothing under its default renderer and hands the
screenshooter a zero-sized frame it dies on, and `--debug`, because without it
the compositor refuses every client the capture protocol. Both were established
against weston 14.0.2 and 15.0.1. The harness asserts all four of these, so a
future edit cannot quietly drop one.

On Wayland there is no window manager to ask for a window class, so that row
asserts something different and says so: the application does not exit under
`GDK_BACKEND=wayland`, which makes a failure to reach the compositor fatal
instead of a silent fall back to X, and the compositor's own screenshot with the
window open differs from the one taken before it started.

One thing is arranged rather than exercised: linger is granted by root before
the engine's own install runs, so the polkit hop is not on the path. The
engine's own suite covers that in both of its shapes. What is being proven here
is packaging.

**The gate does not retry `fermix service install`, on purpose.** The first real
matrix run found a race in the engine's own completion proof: the daemon's
control socket listens before its web endpoint does, and the health probe asked
exactly once, so an install of a perfectly healthy service could be refused with
`health_unavailable`. A retry wrapped around that call here would have hidden it.
A release gate that retries past a product race stops being able to see it, and
this one saw it on its first honest run.

That race is also a lesson about what this gate can and cannot detect. It is
load-dependent: it appeared while the machine was building the rest of the
matrix, and a hundred repeated attempts against the same unfixed engine on a
quiet machine refused not once. So a green row means the failure did not happen,
which on an idle machine is weaker than it looks. When a run of this gate is
used as evidence about a timing bug rather than about packaging, load the machine
deliberately, and measure the unfixed artefact under that same load before
believing a clean result from the fixed one. Otherwise the thing under test is
the rig.

## Building a package by hand, on a machine with only Docker

This is the whole loop on a developer machine with Docker and no Elixir, no
Erlang and no Zig, which is the machine this work is done on. Both checkouts are
siblings.

```sh
export DOCKER_HOST=unix:///var/run/docker.sock

# 1. The engine, out of the engine checkout, with no Elixir on this host.
cd ../fermix
scripts/release/build_app_engine.sh linux_x86_64 /tmp/engine 0.10.5 --container --dev

# 2. The package, against that archive.
cd ../fermix-linux
scripts/build_packages.sh 0.10.5 amd64 --container --engine /tmp/engine/fermix_app_engine_linux_x86_64.tar.gz

# 3. Install it on a machine with a systemd, and watch the window open.
scripts/install_smoke.sh --image ubuntu:22.04 packaging/out/packages/fermix-desktop_0.10.5_amd64.deb

# 4. Or install it for real, on this machine.
sudo apt install ./packaging/out/packages/fermix-desktop_0.10.5_amd64.deb
fermix-desktop --version

# 5. And take it off again.
sudo apt remove fermix-desktop
```

`DOCKER_HOST` is honoured by every script here: each one shells out to `docker`
and none of them sets or unsets it, so a machine with more than one daemon picks
the one that variable names. Step 5 leaves `/var/lib/fermix/runtimes` behind on
purpose, because a still-running engine asks the kernel for that exact file
every time it spawns a helper; `sudo rm -rf /var/lib/fermix/runtimes` after
stopping the engine is how a person takes back the few hundred kilobytes.

`--dev` in step 1 relaxes exactly three release refusals and nothing else: it
allows a dirty checkout, it allows the build source commit to differ from `HEAD`
and it stamps a build id of `dev-<short sha>`. It does not relax the manifest
validation, the loader-digest cross-check or the tree digest, so a `--dev`
archive is still structurally valid and `scripts/verify_engine.sh` still refuses
it against a real pin, which is what keeps it out of a release. `--engine` in
step 2 is refused under `CI=true` for the same reason.

The architecture must be the machine the container runs on: there is no cross
build, and each architecture is built on its own runner.

The first run pulls the private runtime image rather than building it, because
the published image for the current `RUNTIME.lock.json` exists. A developer who
changes that lock file runs `packaging/runtime/build_runtime.sh --container`
once, which takes about forty minutes on a laptop and is then cached.

In CI the same script runs with three variables the action sets:
`FERMIX_RUNTIME_IMAGE`, the published toolkit image for the key above;
`FERMIX_ENGINE_DOWNLOAD_DIR`, which defaults to `packaging/out/engine` and is
where the fetched archives are placed; and `CI=true`, which is what makes
`--engine` a refusal, so a release can only ever be built from a pinned,
verified engine. The packages land in `packaging/out/packages/`.

`build_packages.sh` runs, in order: the shell gates, `cargo fmt --check`,
`cargo clippy -D warnings`, the whole test suite including the widget tests under
`xvfb`, the release build, the staging tree with the verified engine unpacked
into it and the private runtime copied in, `desktop-file-validate`,
`appstreamcli validate --no-net`, `scripts/check_private_runtime.sh`,
`scripts/check_copyright.sh`, both nFPM runs and `scripts/package_dependencies.py`.
It is the whole release gate, which is why the release workflow needs no
separate one.

**`appstreamcli` runs straight through, with nothing accepted.** A warning or an
error fails the build. The version pinned in the build container reports exactly
one thing about this metainfo, and it is a pedantic hint the run does not ask
for: `cid-contains-uppercase-letter`, because `io.tezra.Fermix` carries a capital
F, which is the convention every GNOME application follows and which M38 section
5.3 settles. The metainfo deliberately carries no screenshots until the reference
captures in `docs/design/captures/` have a reviewer's decision against them, and
a software centre is the last place to publish an unreviewed picture; this
validator does not report their absence at all. If a later version reports either
of those, the build fails loudly and a person decides, which is better than a
rail that quietly accepts a list.

## The dependency declaration, and the one thing it cannot prove

nFPM runs no `dpkg-shlibdeps` and no rpm automatic-`Requires` generator: it
writes the relations the template lists and nothing else. The question
`scripts/package_dependencies.py` asks is now the opposite of the one it used to
ask. It used to prove that the declared toolkit relations covered what the binary
needed. It now proves that **no** `NEEDED` entry anywhere in the package resolves
outside `/usr/lib/fermix-desktop` except to a declared host relation, and that
the glibc floor the ELFs require is not above the declared `libc6 (>= 2.34)`.

What it cannot prove is that the binary runs on a host at the floor. That is the
`almalinux:9` and `ubuntu:22.04` rows of the smoke, and the S2 row of
`docs/ACCEPTANCE_RUNBOOK.md`.

## The written offer for the bundled sources

Every release publishes `fermix_desktop_runtime_sources_<version>.tar.gz`: all
twenty-nine locked source tarballs exactly as they were fetched, every patch,
`RUNTIME.lock.json`, `Dockerfile.runtime`, `build_runtime.sh` and a README with
the three commands that rebuild the runtime from the archive alone with no
network. That is how LGPL 2.1's source obligation is honoured by a link rather
than by post, and it is signed with the same identity as the packages, because a
source offer nobody can verify is not an offer.

`packaging/runtime/package_sources.sh` checks every tarball against the lock
file's own sha256 whether it came off the network or out of a local cache, so a
file of the right name with the wrong bytes is refused rather than published as
a source offer. A server whose answer does not match the locked digest is
refused by name rather than retried, because a retry there is asking the same
wrong answer three times; a connection failure retries three times, five seconds
apart, and then names the component and stops. The archive is reproducible:
sorted entries, zeroed mtimes, numeric owners, `gzip -n`.

**It depends on the lock file and nothing else.** The archive is assembled in
the `sign` job by `packaging/runtime/package_sources.sh`, which fetches whatever
it does not already have from each component's locked URL and checks every file
against the recorded sha256 on both paths. Building it from a populated cache
and building it from an empty directory produce byte-identical archives, which
has been measured rather than asserted.

There is deliberately no `actions/cache` restore in front of it, and the reason
is worth keeping. A cache entry is evicted after seven days unread, and this
lock file sits unchanged for months at a time, so a cache would be cold on
exactly the releases that matter. A fast path that is usually cold is not a fast
path; it is a second code path that is rarely exercised and is therefore the one
that breaks. The fetch costs a few minutes of downloading in a release that is
already building packages.

The version is an argument to that script, which enforces its shape and not its
value, so the rail passes the same version the packages were built from and then
requires that exact file name. A tarball for another version is a refusal. That
also means the archive's own sha256 is a per-release fact rather than a constant
— the version appears in the directory names inside it — so publish the digest
on the release page if you like, but never pin it in a gate.

**What is proven about the archive, and what is not.** Its layout mirrors the
repository, so `build_runtime.sh` inside it finds its lock file, its patches,
its Dockerfile and the library it sources at the paths it expects, and its
README gives the command that works from the top of the unpacked archive:

```sh
FERMIX_RUNTIME_SOURCES="$PWD/sources" packaging/runtime/build_runtime.sh --container
```

**A recipient given only this archive and the toolchain image reproduces the
published runtime exactly.** That is not a design intention; it has been run.
The build took its scripts, its lock file, its patches and its Dockerfile from
the unpacked archive, with `FERMIX_RUNTIME_SOURCES` pointed at the archive's own
`sources/`, so not one component tarball came off the network. Fresh volumes, a
directory outside the repository. `compare_manifest` reported the rebuild
identical across all 1493 entries, and all three artefacts — the runtime tar,
the development tar and the manifest — matched the published bytes exactly, at
the same runtime key.

Two conditions belong in that sentence and neither is a hedge. The **toolchain
image** still needs the network: `Dockerfile.runtime` installs from dnf and pip,
which is what the archive's README says and what the offer therefore promises.
And this is **one machine**: cross-machine reproduction has not been tested, so
the claim is that the archive rebuilds the runtime, not that any two hosts agree.
The job count is deliberately not a condition — output that depended on a
machine's core count would be a defect, and one was found and removed on the way
here. An earlier rebuild differed in exactly one file, librsvg's pixbuf loader,
by thirty-six bytes of annobin metadata in a non-loadable section whose size
varies with the number of build jobs. Pinning the job count was rejected as a
fix; the sections are stripped from every private ELF instead.

Two defects that only running it could find are the reason this is worth doing
at all rather than inspecting a file listing: the archive was laid out flat, so
its own build script could not resolve its inputs, and it omitted
`write_manifest.py` and `compare_manifest.py`, so it compiled all twenty-nine
components, passed every check and died on the last step. `package_sources.sh`
now refuses to build an archive missing any file the build needs. **The archive
is verified by running it, not by looking at it.**

**Check the archive's own digest, not only what is inside it.** A per-file check
of an unpacked tree is not the same check: a runtime whose tree reproduced
perfectly, file for file, still produced a development tarball that did not
reproduce, because `gdbus-codegen`'s `__pycache__` survived in it and a `.pyc`
records the interpreter and the source path that wrote it. The tree was correct
on both runs; only comparing the archive's own sha256 found it. That is the
digest a release publishes and a downloader verifies, so it is the digest that
has to be reproducible — the rule generalises past this runtime and is worth
applying to any archive this project starts publishing.

## Which runtime is this, exactly

Every installed tree carries
`/usr/lib/fermix-desktop/share/fermix-desktop-runtime/identity.json`, naming the
runtime cache key, the sha256 of the lock file it was built from, and the GTK,
libadwaita and GLib versions. It is listed in the runtime manifest and checked by
`build_runtime.sh --verify`.

It exists because versions do not identify a build. Three different runtimes all
answer "gtk 4.16.7, libadwaita 1.6.9", so a version comparison cannot tell them
apart, and a stale prefix in a build image did in fact ship past two guards
before it was caught by hashing prefixes by hand. The tree now states its own
identity rather than being interrogated about it.

For support that is the first file to ask a reporter for: it says exactly which
runtime they are running, which no version string can.

**Which engine is in this package, and the ladder for answering it.**
`/usr/share/fermix-desktop/build.json` names `engine_build_id` and
`engine_tree_sha256`, and the second exists because the first can lie: a dirty
working-tree build keeps `dev-<commit>-dirty` while the code under it changes,
so two engines that behave differently can claim one name. The smoke therefore
reads and records the tree digest per row.

The tree digest has a floor of its own, and it was reached in practice. It says
two packages carry different engine trees; it cannot say whether the difference
is a build-id string or a behaviour change, because from outside a rebuilt
binary those look alike — same `.text` size, a different hash, a few hundred
bytes more `.rodata`, a compressed payload that yields nothing to inspection. So
the ladder is: `engine_build_id`, then `engine_tree_sha256`, then the contents,
and the first two narrow the question rather than answering it.

Getting underneath the digest means making the artefact unpack itself. The
engine is a Burrito binary: run it once and the payload lands under
`~/.local/share/.burrito/fermix_linux_package_erts-<erts>_<version>/lib/`, where
the compiled modules compare directly between two engines whose wrapper binaries
differ. That is how "does this package carry the install health-probe fix" was
settled — `Elixir.Fermix.CLI.Service.Packaged.beam` byte-identical to the
engine known to carry it, 619 of 626 modules identical, and the differing seven
all off the install path.

That last rung is the one that fails silently, and it fails in both directions.
The rungs above it read a field out of a file and cannot flatter you; a search
for a symbol inside an artefact can, because a large binary holds a great deal of
plausible-looking text and the answer you were hoping for is one `grep` away. So
**include a name you know is not there**: one extra line, and it is what
separates finding the fix from finding a string. Guard the other direction too —
one attempt at this reported both functions absent because `strings` is not
installed in `debian:12` and the shell swallowed the failure. A missing tool and
a missing symbol look identical in the output, and anyone checking a claim they
expected to be false would have stopped there with their confirmation in hand.

**And there is a rung below that one: a module is not a method.** Finding
`Migration` compiled into an engine says the code was built in, not that a
client can call anything. The management surface is published by `hello`, whose
result carries `capabilities.methods`, and a verb absent from that list does not
exist as far as any caller is concerned — the module can be present for a whole
build cycle before the wiring that exposes it lands. This was got wrong here:
a new module was reported, taken as a working verb, and the owner was nearly
told that the file store had become reversible on a build whose `hello`
published only `secret.set` and `secret.clear`. So when the question is "can
this build do X", ask the running engine what it publishes:

```sh
# from an installed machine, as the account that owns the service
printf '%s' "$request" | … # 4-byte length-prefixed JSON, method "hello"
```

The ladder in full, then: `engine_build_id`, the tree digest, the compiled
modules, and the published method list — each one narrower than the last, and
only the last one answers what a client can actually do.

## The bundled components, and who watches them

Bundling GTK and everything under it means owning the advisories a distribution
would otherwise patch. `.github/workflows/runtime-watch.yml` runs weekly and on
demand: `scripts/runtime_watch.py` reads every component of
`packaging/runtime/RUNTIME.lock.json`, asks OSV what is known about that name and
version and release-monitoring.org what upstream has released, and opens or
updates one issue per component that has something to say. It opens nothing when
there is nothing to say, and a component neither source has a name for is itself
a finding rather than a silence.

GTK and libadwaita are watched inside their own series, because the crate's
feature floor pins 4.16 and 1.6 and a move to the next series is a deliberate
change to `Cargo.toml`, the lock file and the design together.

Nothing is bumped automatically: a point release can change rendering and the
reference captures are a reviewed artifact. A bump is a change to
`RUNTIME.lock.json`, which changes the runtime image key, rebuilds the image
through `runtime.yml`, and is published as the next `<engine version>+<n>`.

## The icons

The eight rasters, the scalable icon and its symbolic pair are checked in under
`packaging/icons/hicolor/`, rendered from the application's own source SVG:

```sh
scripts/render_icons.sh --container
```

`build_packages.sh` verifies the set rather than re-rendering it, so the bytes a
reviewer looked at are the bytes that ship and a release does not depend on
whichever librsvg a build host happens to carry. A change to
`App/Fermix/resources/icons/` is followed by a run of the script above and a
commit of what it wrote; `tests/packaging.rs` fails if the two drift apart.

## When something fails

*The dispatch never arrives.* The engine release is fine and no desktop release
exists. Re-running `dispatch-linux-desktop` from the engine's Actions tab is the
whole retry, and it is idempotent because the branch name is derived from the
tag: a second dispatch updates the same branch and the same pull request.

*The bump pull request's checks fail.* This is the case that matters, because it
means the new engine and the current application disagree about something.
Nothing is published. A person fixes it on `engine/vX.Y.Z` and auto-merge
proceeds. The engine release stays published; there is no rollback of an engine
because of a desktop failure.

*The tag run fails after the tag exists.* Re-running the failed job is the first
move. If the tag itself is wrong, delete it and push it again, which is safe
because `publish` has not run and no asset exists. Once a release is published
it is never re-cut: `version` and `publish` both refuse a tag that already has a
published release, and the fix is the next version.

## Before announcing

Walk `docs/ACCEPTANCE_RUNBOOK.md`. Its rows need a real graphical session on one
GNOME host, one Plasma host and the Pop!_OS 22.04 machine, and no container gate
substitutes for them.

# Fermix for Linux

A native GTK4 and libadwaita client for the Fermix engine, written in Rust.

Fermix itself is a background service: the `fermix` package installs it and your
own systemd user manager runs it. This application is the window you set it up
and keep an eye on it from. It contains no engine, writes no unit, reads no
configuration file and holds no secret code. Everything it shows and changes
goes over `daemon.sock` through the management protocol, and everything that has
to work while the daemon is stopped goes through five typed operations of the
packaged `fermix` command line.

The design is `MILESTONE_38_LINUX_COMPANION_APP.md` in the engine repository.
The rules for this application are `docs/design/LINUX_DESIGN_SYSTEM_REDLINES.md`,
and the build order is `docs/design/IMPLEMENTATION_BRIEF.md`.

## Layout

```
App/Fermix/               the crate: Cargo.toml, build.rs, src/, resources/, contracts/, tests/
packaging/                everything the package installs, the nFPM configuration, both containers
engine/PIN.json           the engine release this desktop version is paired with
scripts/                  the gates, each with a *_test.sh beside it
docs/design/              the redlines, the brief, reviewed captures
docs/RELEASING.md         how a version reaches a Linux desktop
docs/ACCEPTANCE_RUNBOOK.md  the gates that need a person and a real desktop
.github/workflows/        app.yml and contract.yml (every push), release-fermix-desktop.yml (tags)
```

## Building it

The toolkit floor is GTK 4.16 and libadwaita 1.6, and only the floor's features
are enabled, so a symbol above the floor is a compile error rather than a
missing symbol on somebody's machine.

```sh
cd App/Fermix
cargo build
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

On a Mac with Homebrew's GTK, `export PKG_CONFIG_PATH=/opt/homebrew/lib/pkgconfig`
first if `pkg-config` does not find the toolkit.

## Running it against the fixture daemon

There is a second binary, `fixture-daemon`, which answers the management
protocol's own golden responses over a real Unix socket with the real framing.
It is how the application is developed and captured without a Fermix installed.

```sh
cd App/Fermix

# One terminal: the daemon. It prints the socket path when it is ready.
cargo run --bin fixture-daemon -- /tmp/fermix-fixture

# Another: the application, pointed at that home.
FERMIX_DESKTOP_FIXTURE_HOME=/tmp/fermix-fixture \
FERMIX_DESKTOP_CLI="$PWD/tests/fixtures/cli/fermix" \
  cargo run --bin fermix-desktop
```

Both variables are read in debug builds only. A release build uses
`/usr/bin/fermix` and the home binding `fermix service status` reports, and
cannot be pointed at a fixture.

`FIXTURE_DAEMON_SCENARIO` picks what the daemon answers:

| Scenario | What it answers |
|---|---|
| `default` | The published goldens as they are |
| `setup_required` | Readiness is setup required, with one gating personalization failure |
| `restart_pending` | A restart is required, with the daemon's own two reasons |
| `external_change` | The settings file was changed outside Fermix, and every write refuses |
| `unreadable` | The settings file cannot be parsed, and no reload is offered |
| `not_running` | Nothing listens at all |
| `doctor_healthy` | A Doctor run that finished with nothing to do |
| `doctor_failed` | A Doctor run that finished with a failure, its remediation and its evidence |
| `logs_empty` | A log with nothing in it |
| `integrations_states` | A plugin installed and waiting for a sign-in, beside the published rows, and an install job that finishes on the third read |
| `meetings_signed_out` | The notetaker is there and signed out |
| `meetings_absent` | The notetaker is not installed, so the sign-in state is unanswered |
| `meetings_refused` | The notetaker probe was refused, which is also unanswered |
| `computer_states` | The computer-use helper is not installed, and its install finishes on the third read |
| `computer_wayland` | The same, taken in a Wayland session |
| `onboarding_welcome` | A fresh account: nothing answers, so the assistant opens on Welcome |
| `onboarding_starting` | Everything answers but the web door, so the ladder stays on the row that is asking it |
| `onboarding_linger_denied` | The same home, paired with a command line that refuses to enable linger |
| `onboarding_boot_failed` | The same home, paired with a service that was switched on and did not answer |
| `onboarding_connect_ai` | A home with no provider, which is the one decision the assistant will not walk past |
| `onboarding_about_you` | A home whose provider is connected and whose personalization is not |
| `onboarding_refused_personalization` | The same, where the daemon refuses what About you writes |
| `onboarding_no_restart` | A write that lands with no restart owed, and something still owed on a pane the assistant has no screen for |
| `onboarding_restart_needed` | A home that owes nothing but the restart the daemon is asking for |
| `onboarding_ready` | A finished home with one advisory failure left for Home |
| `onboarding_skew` | The same finished home, read against a command line reporting a newer engine installed than the one running |

The assistant's scenarios are the command line's as much as the daemon's: what
the ladder shows is what `service install` and `restart` answer, so
`scripts/capture.sh` pairs each of them with the state that answers it. The
fixture daemon also serves `GET /health/live` on the loopback port its
scenario's own `hello` names, which is the other half of the assistant's finish
gate; a scenario whose `hello` names port 0 has no web door at all, which is how
the ladder is shown still asking for one.

```sh
FIXTURE_DAEMON_SCENARIO=restart_pending cargo run --bin fixture-daemon -- /tmp/fermix-fixture
```

The fake `fermix` under `App/Fermix/tests/fixtures/cli/` stands in for the
packaged command line. It answers each of the five typed operations from the
state directory `FERMIX_FAKE_CLI_STATE` names; the states live beside it.

## Taking the reference captures

The application renders its own reference states to PNGs, through the real
widgets at the real window size, against the fixture daemon. It is how a surface
is reviewed against the redlines without anyone having to describe what they
saw.

```sh
scripts/capture.sh                 # every scenario, light and dark
scripts/capture.sh doctor_failed   # one scenario, both schemes
```

The files land in `docs/design/captures/` and every one of them gets a row in
`docs/design/captures/INDEX.md` carrying the fixture, the commit, the toolkit
versions, the window size, the text scale and the colour scheme. The reviewer
and decision columns are filled in by a person: a capture that exists is not a
capture that was accepted.

The mode itself is `fermix-desktop --capture DIR`, in a debug build only. It
takes the states the running `FIXTURE_DAEMON_SCENARIO` can actually show, and
quits.

## Running the container build

The development machine is usually not a Linux desktop, and the runner images
carry a GTK below the floor, so the build that matters happens in a container.
It needs Docker and takes fifteen to thirty minutes the first time.

```sh
scripts/container_build.sh            # build the image if needed, then gate
scripts/container_build.sh --rebuild  # rebuild the image first
scripts/container_build.sh --shell    # a shell inside it
```

Inside it, `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test` and the
widget tests under `xvfb` all run against the toolkit floor.

## The gates

Each of these has a `*_test.sh` beside it that drives its refusals against a
throwaway copy, because a gate nobody has seen fail is a gate nobody knows
works.

| Gate | What it holds |
|---|---|
| `scripts/verify_contract.sh` | Every vendored contract is byte-identical to the engine's, pinned by two records that have to agree |
| `scripts/check_app_identity.sh` | One identity string, in every place that carries it |
| `scripts/check_copy.sh` | The copy catalogue: its casing column, its forbidden substrings, and no word a surface shows written as a literal |
| `scripts/check_vendor_marks.sh` | Every vendor mark ships with its provenance record, and the roster is the one the daemon publishes |
| `scripts/container_build.sh` | The crate builds and every gate passes on the toolkit floor |
| `scripts/build_packages.sh` | Both packages build from one configuration, and their declared relations cover what the binary actually needs |
| `scripts/verify_engine.sh` | The engine packages a release republishes are the ones `engine/PIN.json` names |
| `scripts/install_smoke.sh` | Both packages install on a machine with a systemd, and the window opens against a real daemon |

## The contract with the engine

There are two contracts, because this application reads two wires.
`App/Fermix/contracts/management/` is a byte-identical copy of the engine's
`apps/fermix_core/priv/management/`, which is what the daemon answers over
`daemon.sock`; `App/Fermix/contracts/cli/` is a byte-identical copy of
`apps/fermix_core/priv/cli/`, which is the typed `--json` verbs of the packaged
command line and the only thing that answers while the daemon is stopped. Both
are pinned by `CHECKSUMS.txt` and `SOURCE.json`. Never edit a vendored file by
hand: copy the tree again from a committed engine commit, regenerate both
records, and verify with

```sh
scripts/verify_contract.sh --source ../fermix
```

The version window and every per-method minimum are read out of the management
schema at compile time, so the client and the tests read the same numbers the
daemon is tested against. The command line's thirty-five goldens are decoded by
`tests/contract.rs` through the same structs the application uses, and the fake
`fermix` under `tests/fixtures/cli/` answers those goldens' bytes and nothing
else.

## Installing it

Fermix for Linux is two packages, and they move together. `fermix` is the
assistant itself, a background service your own systemd runs; `fermix-desktop` is
this window. The window declares an exact-version dependency on the engine, so
install the two from the same release page in one command.

```sh
# Debian, Ubuntu, Mint, Pop!_OS, elementary
sudo apt install ./fermix_<version>_$(dpkg --print-architecture).deb \
                 ./fermix-desktop_<version>_$(dpkg --print-architecture).deb

# Fedora, RHEL, CentOS Stream, Alma, Rocky
sudo dnf install ./fermix-<version>-1.$(uname -m).rpm \
                 ./fermix-desktop-<version>-1.$(uname -m).rpm

# openSUSE and SLE
sudo zypper install ./fermix-<version>-1.$(uname -m).rpm \
                    ./fermix-desktop-<version>-1.$(uname -m).rpm
```

Then open Fermix from the launcher, which takes you through setup. On a machine
with no desktop, run `fermix setup` in a terminal instead and install the
`fermix` package alone.

Every file on a release page is cosign-signed and carries its own `.sha256`,
`.sig` and `.pem`; the release page's own `INSTALL.md` gives the exact
`cosign verify-blob` command for that release. The toolkit floor is GTK 4.16 with
libadwaita 1.6, and the packages declare it, so a host below the floor refuses
the install rather than failing when the window is opened.

## Releasing it

`docs/RELEASING.md` is the procedure, and the order in it is not optional: the
engine releases first, the engine pin bump lands here as a pull request, and only
then is `fermix-desktop-vX.Y.Z` tagged. Both packages of a version become visible
together, because each pins the other exactly.

The desktop version is the version of the engine it pairs with, so this crate is
at the engine's version rather than counting on its own, and a desktop release
carries the number of the engine release its packages depend on exactly.

```sh
# Both packages, for this machine's architecture, in the build container.
FERMIX_DESKTOP_BUILD_ID=local-1 \
FERMIX_DESKTOP_SOURCE_COMMIT=$(git rev-parse HEAD) \
  scripts/build_packages.sh 0.10.4 arm64 --container

# Install them on a machine that has a systemd, and watch the window open.
scripts/install_smoke.sh <engine.deb> packaging/out/fermix-desktop_0.10.4_arm64.deb
```

Neither package may ever carry a Debian revision or an rpm epoch, so a version
containing `-` or `:` is refused before anything is built, and a prerelease tag
produces no packages at all. A packaging-only fix is published as the next
version.

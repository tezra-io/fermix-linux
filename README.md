# Fermix for Linux

A native GTK4 and libadwaita client for the Fermix engine, written in Rust.

Fermix itself is a background service that your own systemd user manager runs.
This application is the window you set it up and keep an eye on it from, and the
two ship as one package: `fermix-desktop` carries the window, the engine and a
private GTK4 runtime, so one install is the whole product. The window still
contains no engine of its own, writes no unit, reads no configuration file and
holds no secret code. Everything it shows and changes goes over `daemon.sock`
through the management protocol, and everything that has to work while the
daemon is stopped goes through five typed operations of the packaged
`/usr/bin/fermix` command line, which is the same binary the engine-only
`fermix` package installs at the same path.

The design is `MILESTONE_38_LINUX_COMPANION_APP.md` in the engine repository.
The rules for this application are `docs/design/LINUX_DESIGN_SYSTEM_REDLINES.md`,
and the build order is `docs/design/IMPLEMENTATION_BRIEF.md`.

## Layout

```
App/Fermix/               the crate: Cargo.toml, build.rs, src/, resources/, contracts/, tests/
packaging/                everything the package installs, the nFPM configuration, the containers
packaging/runtime/        the private GTK4 runtime: its lock file, its build, its manifest
engine/PIN.json           the engine release this package carries
scripts/                  the gates, each with a *_test.sh beside it
docs/design/              the redlines, the brief, the single package amendment, reviewed captures
docs/RELEASING.md         how a version reaches a Linux desktop
docs/ACCEPTANCE_RUNBOOK.md  the gates that need a person and a real desktop
.github/workflows/        app.yml, contract.yml (every push), packages.yml, runtime.yml,
                          engine-release.yml, release-fermix-desktop.yml, runtime-watch.yml
```

## Building it

The toolkit floor is GTK 4.16 and libadwaita 1.6, and only the floor's features
are enabled, so a symbol above the floor is a compile error rather than a
missing symbol on somebody's machine. The package carries exactly that: a
private GTK 4.16 and libadwaita 1.6 at `/usr/lib/fermix-desktop`, built from
`packaging/runtime/RUNTIME.lock.json`.

The build that matters therefore happens in a container, where that prefix is at
the same absolute path it is at on a user's machine:

```sh
scripts/container_build.sh
```

Inside it the crate finds the toolkit through
`PKG_CONFIG_PATH=/usr/lib/fermix-desktop/lib/pkgconfig` and needs nothing from
the host. Against a host toolkit, on a machine that happens to have GTK 4.16 and
libadwaita 1.6, the ordinary commands still work and prove less:

```sh
cd App/Fermix
cargo build
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

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
It needs Docker and takes fifteen to thirty minutes the first time. It pulls the
private runtime image for the current lock file rather than compiling it; a
change to `packaging/runtime/RUNTIME.lock.json` needs one run of
`packaging/runtime/build_runtime.sh --container`, which takes about forty
minutes and is then cached.

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
| `scripts/build_packages.sh` | Both families build from one configuration, and nothing in the package links outside the private prefix except a declared host relation |
| `scripts/check_private_runtime.sh` | Every private object resolves through `$ORIGIN/../lib`, and no private object names a host GTK, GLib or pango |
| `scripts/check_no_network.sh` | The application ELF names no symbol from GIO's TLS or resolver surface, so the claim that the window opens no connection is a gate |
| `scripts/check_copyright.sh` | The generated copyright file and the runtime lock file agree, which is what stops a component being bundled without its licence |
| `scripts/verify_engine.sh` | The engine archive the package carries is the one `engine/PIN.json` names, by digest and by cosign identity, and its manifest agrees with what was unpacked |
| `scripts/engine_bump.py` | The dispatch that writes the pin is the engine repository's own, field by field |
| `scripts/asset_name.sh` | The name an asset is published under is the name a downloader asks for |
| `scripts/runtime_watch.py` | Every bundled component is watched for advisories and for upstream releases |
| `scripts/install_smoke.sh` | The package installs on a machine with a systemd, on the oldest and newest release of each family, and the window opens against a real daemon on X11 and on Wayland |

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

Fermix for Linux is one package. `fermix-desktop` carries the window, the
assistant itself and the toolkit the window draws with, so one install is the
whole product and there is nothing to pair it with.

```sh
# Debian, Ubuntu, Mint, Pop!_OS, elementary
sudo apt install ./fermix-desktop_<version>_$(dpkg --print-architecture).deb

# Fedora, RHEL, CentOS Stream, Alma, Rocky
sudo dnf install ./fermix-desktop-<version>-1.$(uname -m).rpm

# openSUSE Tumbleweed and SLE
sudo zypper install ./fermix-desktop-<version>-1.$(uname -m).rpm
```

Then open Fermix from the launcher, which takes you through setup. On a machine
with no desktop, install the engine-only `fermix` package from the engine's own
release page instead and run `fermix setup` in a terminal. The two are
alternatives rather than layers: each provides, conflicts with and replaces the
other, so installing either one replaces the other in a single transaction and
your settings survive, because neither package owns them.

Every file on a release page is cosign-signed and carries its own `.sha256`,
`.sig` and `.pem`; the release page's own `INSTALL.md` gives the exact
`cosign verify-blob` command for that release.

The only requirement is glibc 2.34, which every supported release of Ubuntu,
Debian, Fedora and the RHEL family has carried since 2021. The package declares
it, so a host below that floor refuses the install rather than failing when the
window is opened. There is no GTK requirement at all: the package carries its
own GTK 4.16, libadwaita 1.6 and everything under them, which is why it runs on
Ubuntu 22.04 with its GTK 4.6. What it uses from the host is the graphics
driver, the session bus, the X11 client libraries and the installed fonts, and
the package manager brings any of those that are missing.

Removing the package leaves `/var/lib/fermix/runtimes` behind on purpose,
because a running engine asks the kernel for that exact file every time it
starts a helper.

## Releasing it

`docs/RELEASING.md` is the procedure, and almost none of it is typed. An engine
release in `tezra-io/fermix` dispatches here, a bot writes `engine/PIN.json` and
the version after verifying the engine archives it pins, opens an auto-merging
pull request behind the full gate set, and tags `fermix-desktop-vX.Y.Z` when it
lands. A person is involved at the engine tag and at the protected
`release-linux` environment, and nowhere in between.

The desktop version is the version of the engine the package carries, so this
crate is at the engine's version rather than counting on its own. A desktop-only
fix, a toolkit CVE rebuild or a packaging fix is published as `X.Y.Z+N`; neither
package may ever carry a Debian revision or an rpm epoch, so a version
containing `-` or `:` is refused before anything is built and a prerelease tag
produces no packages at all.

On a machine with only Docker, the whole loop by hand, with both checkouts as
siblings:

```sh
export DOCKER_HOST=unix:///var/run/docker.sock

cd ../fermix
scripts/release/build_app_engine.sh linux_x86_64 /tmp/engine 0.10.5 --container --dev

cd ../fermix-linux
scripts/build_packages.sh 0.10.5 amd64 --container --engine /tmp/engine/fermix_app_engine_linux_x86_64.tar.gz
scripts/install_smoke.sh --image ubuntu:22.04 packaging/out/packages/fermix-desktop_0.10.5_amd64.deb
```

`--dev` on the engine build relaxes exactly three release refusals so that
building an engine needs no Elixir on this machine, and `verify_engine.sh` still
refuses the result against a real pin, which is what keeps it out of a release.

# Fermix for Linux: the desktop app

A small GTK4 + libadwaita app, written in Rust. It ships in one package, `fermix-desktop`, with its
own GTK runtime and the Fermix engine. The window is a **client only** of the engine's daemon, which
the same package installs and the `fermix.service` user unit runs: the app talks to it over
`~/.fermix/daemon.sock` (management protocol v2). It holds no config and no secrets.

## Layout

```
core/        fermix-client: socket framing, management client, job polling rules, and every
             sentence a screen shows (view.rs). No GTK, so it builds and tests on any host.
app/         fermix-desktop: the window. Pages draw from State; app.rs/flows.rs/service.rs
             turn window actions into daemon calls.
app/marks/   vendor marks and the Fermix brand files, byte for byte, with PROVENANCE.json
rive/        fermix-rive: plays the pet's Rive animation with the vendored Rive C++ runtime
data/        desktop entry, D-Bus service, metainfo and icons
docs/        the design record the code cites, kept in a private design repo (docs is a link to it)
engine/      PIN.json: the signed engine release the package carries
packaging/   the package build's container half, the nFPM template, the manual page, the lint
             overrides, the copyright generator, and the private GTK runtime (packaging/runtime/)
scripts/     dev.sh; the package build, its gates, the engine pin and its verification, the lint
             and the install smoke, each with a *_test.sh beside it
```

## Develop

The app needs GTK 4.22 and libadwaita 1.9, which most hosts do not have, so the dev loop builds in
the GNOME 50 SDK and runs on the GNOME 50 runtime. It needs `flatpak`, the SDK, the runtime and the
Rust extension:

```
flatpak install --user flathub org.gnome.Sdk//50 org.gnome.Platform//50 org.freedesktop.Sdk.Extension.rust-stable//25.08
scripts/dev.sh check    # rustfmt, clippy -D warnings, tests
scripts/dev.sh run      # build (incremental) and run against your daemon
scripts/dev.sh build    # build only
scripts/dev.sh fmt      # apply rustfmt
cargo test -p fermix-client   # the core also tests on the host
FERMIX_HOME=/nowhere scripts/dev.sh run   # see the not-running screens safely
```

Done means `scripts/dev.sh check` is green, with zero warnings. CI runs the same check.

`dev.sh run` runs the window in a sandbox. `~/.fermix` is mounted **read-only**: connecting to a
socket needs no write access, and the app must never touch the config, secrets or memory files
beside it. `org.freedesktop.systemd1` is reachable so "Start Fermix" and "Run in the background"
can start, enable or stop the user unit, and `org.freedesktop.login1` so the service can stay on
after logout (linger). `~/.config/fermix` is read-only, to know whether the package's binding is
there. The PulseAudio socket carries voice calls. The sandbox has its own home, so "Open at login"
writes its entry there, where the session never reads it; the packaged window keeps
`~/.config/autostart/io.tezra.Fermix.desktop` itself.

The window reports its crate's version, from `app/Cargo.toml`, which is also the package's (see
[Releasing](#releasing)): the About dialog shows it, and the chat connection gives it to the engine.

## Build the packages

`fermix-desktop_<version>_<arch>.deb` and `fermix-desktop-<version>-1.<rpm arch>.rpm` build in
containers, so this needs Docker. Reading the engine's releases needs `gh`, logged in, and verifying
them needs `cosign` on `PATH` (`scripts/verify_engine.py` names the version). Each script's header
says what it takes and what it checks.

1. The private GTK runtime, from `packaging/runtime/RUNTIME.lock.json`
   ([`packaging/runtime/README.md`](packaging/runtime/README.md)). Its outputs go to
   `$FERMIX_RUNTIME_OUT`, by default `~/.cache/fermix-desktop-runtime/out`.

   ```
   packaging/runtime/build_runtime.sh --container
   packaging/runtime/smoke_runtime.sh
   ```

2. The packages, carrying the engine `engine/PIN.json` pins. Their version is the app's own, the
   X.Y.Z of `app/Cargo.toml`, which `--print-app-version` prints; a release build refuses any
   other. They provide `fermix` at the pinned engine's version. `--dev` carries a local engine build
   instead, is versioned `<app version>+dev.<time>.<commit>`, and is refused under CI. The
   packages go to `--out <dir>`, by default `~/.cache/fermix-desktop-package/out`.

   ```
   scripts/build_packages.sh --version "$(scripts/build_packages.sh --print-app-version)" [--arch arm64]
   scripts/build_packages.sh --dev <engine .deb> <engine .rpm>
   ```

3. The lint (lintian and rpmlint) and the install smoke, which installs a package on a clean image
   and draws the window there. `--image` takes each system the release claims: ubuntu:22.04,
   24.04 and 26.04, debian:12 and 13, fedora:44, and almalinux:9 and 10. With `--upgrade-from`
   and an earlier package, the smoke instead upgrades to the package from that one, over its
   engine running as a user service, on ubuntu:24.04 or fedora:44. That container runs systemd,
   so it gets CAP_SYS_ADMIN to make its cgroups, and AppArmor does not confine it. It is not
   privileged.

   ```
   scripts/lint_packages.sh <deb> <rpm> --out <dir>
   scripts/install_smoke.sh --image ubuntu:22.04 <deb>
   scripts/install_smoke.sh --image fedora:44 <rpm>
   scripts/install_smoke.sh --image ubuntu:24.04 <deb> --upgrade-from <earlier deb>
   ```

To carry a new engine release, `scripts/engine_pin.py <tag>` writes `engine/PIN.json` from that
release of `tezra-io/fermix`. That changes what the package ships, so it goes out under a new app
version.

The packaging tests are the `*_test.sh` files beside the scripts. A few run a tool in a container,
from an image already pulled and with no network. `release.yml` runs every one of them, and the
runtime's own `packaging/runtime/build_runtime_test.sh`, which reads the sources
`build_runtime.sh --fetch` downloads:

```
for test in scripts/*_test.sh packaging/*_test.sh; do bash "$test"; done
```

## Releasing

`.github/workflows/release.yml` builds what a release ships on every push to `main` and every
pull request that can change a package. For amd64 and arm64 each, it builds the runtime from
the lock file, smokes it, and builds both packages. Then it lints them, installs them on every
supported system, and runs the packaging tests. It also upgrades the latest published release to
them on ubuntu:24.04 and fedora:44, and writes an update feed from them signed with the test key of
`core/tests/fixtures/update-feed`. A release adds a tag to that.

The app learns of a release from its update feed, `https://fermix.ai/linux/appcast.json`. The
format is set out in `packaging/update_feed.py`, which writes it, and which the app's reader
follows. The feed is signed with ed25519. `packaging/update-keys.json` holds the public keys the app
compiles in: the `current` key, which signs, and a `standby` key. The `release-linux` environment
holds the current key's private half as `LINUX_FEED_SIGNING_KEY`, a PKCS#8 PEM. To rotate, put the
standby's private key in that secret, make the standby current in `update-keys.json`, and name a new
standby there; installed apps already trust the old standby. A release's packages are its own
release assets, unless the repository variable `LINUX_PACKAGE_BASE_URL` names another https base,
to which the workflow adds `/vX.Y.Z`.

1. The release version is the app's own, X.Y.Z, the `version` in `app/Cargo.toml`, as the macOS
   app's is its marketing version. It does not follow the engine's. A release bumps it there, on
   `dev`, and `Cargo.lock` with it: `scripts/dev.sh check` rewrites the lock's entry, and the
   package build, which runs cargo with `--locked`, refuses a lock that disagrees. Every change to
   what a release ships is a new X.Y.Z: there is no rebuild suffix.
2. Pinning a new engine is a step of its own: `scripts/engine_pin.py <tag>` writes
   `engine/PIN.json`, and `engine_pin.py --check` holds the pin to the published release. It
   changes what the package carries, so it needs a new app version. The packages provide `fermix`
   at the pinned engine's version, and the release notes name it.
3. Write `packaging/notes/X.Y.Z.md`, the lines the app's update dialog shows: what the release
   brings, one `- ` sentence a line, 1 to 8 of them, each at most 300 characters, with `` `code` ``
   the only markup. Optional front matter, between two `---` lines, is where
   `critical: true` goes (the app then offers no Skip), and `minimum_installed_version: X.Y.Z`
   (the app then offers an older install a stepping stone first). The workflow refuses a version
   without notes before it builds anything, and `packaging/update_feed_test.sh` holds every notes
   file to its form.
4. Merge `dev` into `main` through a pull request, with `release.yml` green on it.
5. Tag the merge on `main` as `vX.Y.Z`, the form the macOS app's tags take, and push the tag. The
   repository's tag ruleset limits who can. The workflow refuses a tag that is not the app's
   version, an engine pin `engine_pin.py --check` refuses, and a tag whose release is already
   published.
6. The tag's run signs every asset with cosign under the workflow's identity
   (`.github/workflows/release.yml@refs/tags/vX.Y.Z`) and writes a `.sha256` beside each. Its feed
   job then writes the release's item and merges it into the feed fermix.ai serves. The floor is
   the `appcast.json` of the newest published release that carries one: the served feed must
   verify, carry every release the floor carries, and be no older than it, or the release stops.
   Only while no published release carries a feed does a 404 there start a new one. The job signs
   the result in the `release-linux` environment and verifies it. The run stages it all on a draft
   release with notes from `packaging/release_notes.md`, then downloads each asset back and checks
   it. The assets are both packages for both architectures,
   `fermix_desktop_runtime_sources_X.Y.Z.tar.gz`, `appcast.json` and `appcast-item.json`.
7. Read the draft, then publish it. A published release is never refilled: a fix is the next
   version.
8. Then, and not before, put the release's `appcast.json` where
   `https://fermix.ai/linux/appcast.json` serves it. A draft's asset urls are not public, so a feed
   that named them earlier would offer packages no one can download. Check what is served,
   downloaded to a file:

   ```
   packaging/update_feed.py verify --feed <file> --keys packaging/update-keys.json
   ```

   Until it is served, no installed app learns of the release, and the next release's feed job
   refuses the older feed still served.

## Facts the code relies on (measured against fermix 0.11.0)

- Browser sign-in exists only for `openai_codex` and `xai`. Anthropic's ways in are importing the
  Claude Code login, a `claude setup-token`, or an API key; `auth.start anthropic` is refused.
- A ChatGPT re-sign-in leaves the provider row byte-identical, so the app acknowledges completion
  itself ("Signed in just now", a toast) instead of waiting for the row to change.
- A job does not say which provider it belongs to, so a sign-in started before the app restarted
  cannot be re-attached to its row.

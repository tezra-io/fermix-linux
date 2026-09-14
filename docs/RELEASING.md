# Releasing Fermix for Linux

Two packages reach a Linux desktop, and they move together. `fermix` is the
engine and is built and released in `tezra-io/fermix`. `fermix-desktop` is the
window and is built and released here. The window declares
`Depends: fermix (= <version>)` and `Requires: fermix = <version>`, so the two
carry one version number and become visible in one flip.

## The order, and why it cannot be reordered

1. **The engine releases first.** `tezra-io/fermix` cuts `vX.Y.Z`, and its
   `release.yml` publishes `fermix_<version>_<amd64|arm64>.deb` and
   `fermix-<version>-1.<x86_64|aarch64>.rpm`, each cosign-signed with its
   `.sha256`, `.sig` and `.pem` beside it.
2. **The pin bump lands here, as a pull request.** `engine/PIN.json` names that
   release: the tag, its source commit, the certificate identity that tag signs
   as, and each of the four packages with its digest. Either every one of those
   is filled or every one is null; `scripts/engine_pin.sh` refuses the half
   filled state, because a half-filled pin reads as a pin and names an engine
   nothing can verify.
3. **The version bump lands here, in the same pull request or the next one.**
   `App/Fermix/Cargo.toml` and `packaging/io.tezra.Fermix.metainfo.xml` both take
   the engine's version. `scripts/build_packages.sh` refuses a build whose
   version is not all three of the tag's, the crate's and the pin's.
4. **The tag is pushed here**, `fermix-desktop-vX.Y.Z`, and
   `release-fermix-desktop.yml` does the rest.

A window in which `fermix` N+1 is visible and `fermix-desktop` N+1 is not holds
every desktop install back on the old engine, because apt holds `fermix` rather
than break the pin, and to a person that looks like an outage. That is the reason
step 2 is a pull request rather than a step of the release.

## The version rule, and what it costs

Neither package may ever carry a Debian revision or an rpm epoch. Both spellings
of the exact-version relation have to mean the same thing, and they stop meaning
the same thing the moment either side is rebuilt as `0.9.0-2`: in dpkg the `=`
relation compares the full version including the revision, so the relation
becomes unsatisfiable and `fermix-desktop` is uninstallable; in rpm a
version-only `=` matches any release until an epoch is introduced and then stops
matching.

So `scripts/build_packages.sh` and the release workflow both refuse a version
containing `-` or `:` before anything is built, and **a prerelease tag produces
no packages at all**. A packaging-only fix is published as the next version,
never as a revision of the current one.

One asymmetry follows and is the same one the engine package ships with: nFPM's
`release` field is a single top-level value, so the two families cannot be given
different revisions from one configuration. It is left empty, which produces a
revision-free Debian version, and **nFPM's rpm packager defaults an empty release
to `1`**. The built files are therefore:

```
fermix-desktop_<version>_<amd64|arm64>.deb
fermix-desktop-<version>-1.<x86_64|aarch64>.rpm
```

That asymmetry is harmless, because an rpm `=` relation on a version matches any
release.

## What the rail does

| Job | Does |
|---|---|
| `version` | Reads the version out of the tag and refuses anything either family would read differently, then holds it equal to `Cargo.toml` and to the metainfo's release entry. Reports whether the engine pin is pinned |
| `build` | One runner per architecture, `ubuntu-24.04` and `ubuntu-24.04-arm`, each running `scripts/build_packages.sh <version> <arch> --container` |
| `sign` | `cosign sign-blob` keyless over all four packages, a `.sha256` beside each, and a `verify-blob` of what it just signed |
| `engine` | Only when the pin is pinned: downloads the four engine packages from the engine release and verifies each against the pin |
| `verify` | The install smoke, per architecture: both packages installed on a systemd container, the engine brought up under an ordinary account, the window drawn on a display |
| `publish` | `gh release create --latest=false` with the desktop packages, their sidecars, the verified engine packages when pinned, and `INSTALL.md` rendered from `packaging/INSTALL.md.tmpl` |

`publish` runs in the protected `release-linux` environment. Whoever can push a
matching tag would otherwise sign and publish arbitrary code under the
organisation's identity, so the environment is the review, and the required
reviewer is the point of it. Every action is pinned to an immutable commit SHA.

There is no notarization, no cask and no Sparkle here, and each absence is
deliberate: none of the three has a Linux referent. What remains is the keyless
cosign signature over each artifact, and, when the repository service exists, its
own GPG signature over `InRelease` and `repomd.xml.asc`, which is the signature a
user's package manager actually verifies.

## Building the packages by hand

On a Mac or any host with Docker:

```sh
FERMIX_DESKTOP_BUILD_ID=local-1 \
FERMIX_DESKTOP_SOURCE_COMMIT=$(git rev-parse HEAD) \
  scripts/build_packages.sh 0.10.4 arm64 --container
```

The architecture must be the machine the container runs on: the binary links the
host's GTK, so there is no cross build, and each architecture is built on its own
runner. The packages land in `packaging/out/`, beside `build.json` and the
rendered nFPM configuration.

`build_packages.sh` runs, in order: the shell gates, `cargo fmt --check`,
`cargo clippy -D warnings`, the whole test suite including the widget tests under
`xvfb`, the release build, the staging tree, `desktop-file-validate`,
`appstreamcli validate --no-net`, both nFPM runs, and the declared-against-derived
dependency check. It is the whole release gate, which is why the release workflow
needs no separate one.

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
writes the relations `packaging/nfpm-fermix-desktop.yaml.tmpl` lists and nothing
else. So after the packages exist, `scripts/build_packages.sh` derives what the
built binary actually needs and refuses a package whose declaration does not
cover it:

* every library the binary links directly is either one of the two the
  application is allowed to link (`libgtk-4.so.1`, `libadwaita-1.so.0`), with its
  package declared in both families, or something a declared package brings with
  it;
* every Debian relation `dpkg-shlibdeps` derives names a package that is
  declared or is in the declared set's own dependency closure;
* a package declared here at a minimum version older than the one the binary
  needs is refused;
* the exact-version relation on `fermix` is present in both spellings.

What it cannot prove is that the binary runs on a host at the floor. A relation
covered through another package's dependencies is covered by name, and its
version is whatever that package requires; the glibc floor in particular follows
from the build image, which is why the image is pinned by digest. The proof is an
install on a floor host, which is M38 section 13.1 gate 25 and is a row of
`docs/ACCEPTANCE_RUNBOOK.md`.

## The install smoke

```sh
scripts/install_smoke.sh <engine.deb> <desktop.deb>
```

It stands up a Debian container running systemd as process one, installs both
packages through apt, grants linger to an ordinary account, runs
`fermix service install --json --home "/home/test/fermix home"` and
`fermix service status --json` as that account, and then opens the window on an
Xvfb display and photographs it. The evidence lands in `packaging/out/smoke/`.

Pass `-` as the engine package to run everything that does not need a daemon; the
script then says exactly what it could not run rather than reporting a pass.

One thing is arranged rather than exercised: linger is granted by root before the
engine's own install runs, so the polkit hop is not on the path. The engine's own
suite covers that in both of its shapes. What is being proven here is packaging.

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

## Before announcing

Walk `docs/ACCEPTANCE_RUNBOOK.md`. Thirteen of its rows need a real graphical
session on one GNOME host and one Plasma host, and no container gate substitutes
for them.

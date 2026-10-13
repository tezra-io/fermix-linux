# Fermix for Linux

Fermix for Linux is one package, `fermix-desktop`. It carries the Fermix window, the GTK runtime the
window draws with, and the Fermix engine.

Fermix itself is a background service. The package installs the engine, and your own systemd user
manager runs it as `fermix.service`. The window is a client of that service only: it talks to it
over the sockets in `~/.fermix` and holds no configuration and no secrets of its own. From it you
set Fermix up, chat with it, talk to it, and keep an eye on it.

## Install

Download the package for your system and architecture from a
[release](https://github.com/tezra-io/fermix-linux/releases), then install it with your package
manager.

Ubuntu, Debian and the systems built on them:

```sh
sudo apt install ./fermix-desktop_<version>_amd64.deb     # or _arm64.deb
```

Fedora, RHEL and the systems built on them:

```sh
sudo dnf install ./fermix-desktop-<version>-1.x86_64.rpm  # or .aarch64.rpm
```

Then open Fermix from your applications. It takes you through setup.

### Supported systems

- Ubuntu 22.04
- Ubuntu 24.04
- Ubuntu 26.04
- Debian 12
- Debian 13
- Fedora 44
- RHEL 9, AlmaLinux 9 and Rocky Linux 9
- RHEL 10, AlmaLinux 10 and Rocky Linux 10

Each is supported on amd64 and arm64, in an X11 or a Wayland session; RHEL 10 and its rebuilds have
Wayland sessions only. Systems built on Ubuntu, such as Pop!_OS and Linux Mint, install the Ubuntu
package. The package needs glibc 2.34 or newer and declares it, so apt and dnf refuse it on an older
system. It brings its own GTK, so the version of GTK on the system does not matter.

### Verifying a download

Each file on a release has a `.sha256`, a cosign signature (`.sig`) and its certificate (`.pem`)
beside it. The release workflow signs with its GitHub Actions identity, keyless. Verify a download
with:

```sh
sha256sum --check fermix-desktop_<version>_amd64.deb.sha256
cosign verify-blob \
  --certificate fermix-desktop_<version>_amd64.deb.pem \
  --signature fermix-desktop_<version>_amd64.deb.sig \
  --certificate-identity https://github.com/tezra-io/fermix-linux/.github/workflows/release.yml@refs/tags/v<version> \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com \
  fermix-desktop_<version>_amd64.deb
```

### Moving from the `fermix` package

`fermix-desktop` carries the same engine files as the engine-only `fermix` package, and replaces it.
apt removes `fermix` and installs `fermix-desktop` in one transaction. dnf does the same once it is
allowed to remove a package:

```sh
sudo dnf install --allowerasing ./fermix-desktop-<version>-1.x86_64.rpm
```

A running Fermix service keeps going, and your settings, memory and history stay where they are.
The package's version is the app's own, not the engine's: `fermix --version` says which engine it
carries, and each release's notes name it.

### Moving from the Flatpak

The earlier Flatpak and the package both use the app id `io.tezra.Fermix`, so remove the Flatpak
once the package is installed:

```sh
flatpak uninstall --user io.tezra.Fermix
```

If Open at login was on, it stays on: the first time the window opens, it writes the login entry
again to start itself.

### A machine with no desktop

A machine with no desktop installs the engine-only `fermix` package from
[tezra-io/fermix](https://github.com/tezra-io/fermix/releases) instead.

### Sources

The package's private runtime includes LGPL libraries. Each release carries
`fermix_desktop_runtime_sources_<version>.tar.gz`, which holds the source of every library in that
runtime as it was built: each upstream tarball, the patches, the Rust crates compiled in, the
licence files, and the scripts and container definition that build it.

## Development

Everything is in [`desktop/`](desktop/README.md): how to build, test, run and package it.

```
desktop/     the app: core logic, the window, its packaging, the design record
```

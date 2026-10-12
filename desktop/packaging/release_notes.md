Fermix for Linux @VERSION@ is one package holding the Fermix window, the GTK runtime it draws
with, and the Fermix engine @ENGINE_VERSION@.

## Install

Download the package for your system and architecture from the assets below, then install it with
your package manager.

Ubuntu, Debian and their derivatives:

```sh
sudo apt install ./fermix-desktop_@VERSION@_amd64.deb     # or _arm64.deb
```

Fedora, RHEL and their derivatives:

```sh
sudo dnf install ./fermix-desktop-@VERSION@-1.x86_64.rpm  # or .aarch64.rpm
```

Then open Fermix from your applications.

- Installing fermix-desktop replaces the engine-only `fermix` package if it is installed. The
  engine's files are the same, so a running Fermix service keeps going. apt replaces it on its
  own; dnf needs to be allowed to: `sudo dnf install --allowerasing ./fermix-desktop-…rpm`.
- If you used the earlier Flatpak, remove it: `flatpak uninstall --user io.tezra.Fermix`.
- A machine with no desktop installs the engine-only `fermix` package from
  [tezra-io/fermix](https://github.com/tezra-io/fermix/releases) instead.

## Verify a download

Every asset has a `.sha256`, and a cosign signature (`.sig`) and certificate (`.pem`) from this
release's GitHub Actions workflow:

```sh
sha256sum --check fermix-desktop_@VERSION@_amd64.deb.sha256
cosign verify-blob \
  --certificate fermix-desktop_@VERSION@_amd64.deb.pem \
  --signature fermix-desktop_@VERSION@_amd64.deb.sig \
  --certificate-identity https://github.com/@REPOSITORY@/.github/workflows/release.yml@refs/tags/@TAG@ \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com \
  fermix-desktop_@VERSION@_amd64.deb
```

## Sources

`fermix_desktop_runtime_sources_@VERSION@.tar.gz` holds the source of every library in the
package's private runtime, as it was built: each upstream tarball, the patches, the Rust crates
compiled in, the licence files, and the scripts and container definition that build it.

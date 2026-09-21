# Single Package Amendment

One download installs Fermix on a Linux desktop. The package carries the window, the engine, and the
toolkit the window needs, so it installs and runs on Ubuntu 22.04 as well as on Fedora 44, and the
person who downloads it opens Fermix and the setup assistant starts the engine. Nothing is stitched
together by hand.

This document amends `fermix/docs/design/MILESTONE_38_LINUX_COMPANION_APP (1).md` (M38). Everything
M38 says that this document does not touch still holds: the management protocol, the UI, the
vendored contracts, the fixture daemon, the design system, the copy rules, the accessibility gates,
the acceptance matrix. What changes is the shape of what ships and how it is built and released.

## 1. Decision summary, and what it supersedes

Six decisions.

1. **The desktop product is one package per family.** `fermix-desktop_<version>_<arch>.deb` and
   `fermix-desktop-<version>-1.<arch>.rpm` each contain the window, the engine, and a private GTK4
   runtime. There is no exact-version relation between two packages because there are no longer two
   packages on a desktop.
2. **The engine-only package `fermix` stays, unchanged, for headless hosts.** The two are mutually
   exclusive alternatives, not a stack. `fermix-desktop` provides, conflicts with and replaces
   `fermix`.
3. **The engine's installed layout is byte-identical in both packages.** `/usr/bin/fermix`,
   `/usr/lib/fermix/`, `/usr/lib/systemd/user/fermix.service`, `/usr/share/fermix/engine.json`, the
   runtime payload and the loader store under `/var/lib/fermix/runtimes/<digest>`. One engine
   layout, one Doctor, one CLI path, one set of maintainer-script steps.
4. **The window carries its own GTK4, libadwaita, GLib and everything under them that the host
   cannot supply at the required version**, in a private prefix at `/usr/lib/fermix-desktop/`, built
   on a glibc 2.34 base. The host supplies libc, the graphics drivers, the session libraries and the
   X11 client libraries. Section 4 lists every boundary and why it sits there.
5. **The engine reaches the desktop build as a signed release asset**,
   `fermix_app_engine_linux_<arch>.tar.gz`, mirroring the macOS
   `fermix_app_engine_macos_<arch>.tar.gz` rail exactly: a staged tree, an `engine-manifest.json`,
   maintainer-script fragments, cosign keyless signature with `.sig`, `.pem` and `.sha256` sidecars.
   No `.deb` is ever scraped. `engine/PIN.json` becomes a build input.
6. **Every engine release drives a desktop release automatically.** The engine's `release.yml`
   dispatches to `tezra-io/fermix-linux` after the release is published; a workflow there writes the
   pin and the version from the release's own sidecars, opens an auto-merging pull request behind
   the full gate set, and tags. A person approves once, in the protected `release-linux`
   environment, at publish.

### What this amends

| Superseded or amended | Where | What replaces it |
|---|---|---|
| M38 §2.2, "The two packages" | `fermix/docs/design/MILESTONE_38_LINUX_COMPANION_APP (1).md:393` | Section 3 below. The `fermix-desktop` table gains the engine's files and the private runtime; the `Depends: fermix (= <version>)` relation is deleted |
| M38 §2.2, the version-lock paragraphs and the "no Debian revision, no rpm Epoch" rule | same file, from `:475` | Section 6 below. The rule existed only to keep two spellings of an exact-version relation equal. With the relation gone, desktop-only rebuilds are versioned `<engine version>+<n>` |
| M38 §2.6, "Install experience, per family" | same file, `:851` | Section 3.5 below. One `apt install` or `dnf install` line, not two package names |
| M38 §5.1, the toolkit floor as a host requirement | same file, `:1439` | Section 4 below. The floor is what the package carries, not what the host has |
| M38 §12.1, where each artifact is built, and the declared-dependency gate | same file, `:3520` | Sections 5 and 8 below |
| M38 §13.1 gate 25, "an install on a floor host" | same file, `:3795` | Section 8.4. The smoke matrix replaces one floor host with the oldest and newest of each family |
| `docs/RELEASING.md`, the whole document | `/home/sujshe/src/fermix-linux/docs/RELEASING.md` | Rewritten by slice 6. The four-step manual order becomes the automated sequence of section 6 |
| `docs/ACCEPTANCE_RUNBOOK.md`, every row that installs two packages | `/home/sujshe/src/fermix-linux/docs/ACCEPTANCE_RUNBOOK.md` | Amended in place by slice 6 |
| `CLAUDE.md`, "the `fermix` package installs and systemd runs" | `/home/sujshe/src/fermix-linux/CLAUDE.md:3` | Amended by slice 6 |
| `engine/PIN.json`, schema version 1 | `/home/sujshe/src/fermix-linux/engine/PIN.json` | Schema version 2, section 5.3 |

M38 §2.1 (the channels), §2.3 (the repositories), §2.4 (signing), §2.5 (what is refused), §3 (the
management plane), §4 (service ownership), §5.2 through §5.11, §6, §7, §8, §10 and §11 are not
amended. M38 §2.2's paragraph on why the split exists is still correct about headless hosts; what
changes is that the headless package is now the alternative rather than the base.

## 2. Support matrix

The glibc floor is **2.34**. It is set by the build base image, AlmaLinux 9, and it is the lowest
floor that covers both the oldest deb target and the oldest rpm target in one build.

| Distribution | Version | glibc | Session | CI | Note |
|---|---|---|---|---|---|
| Ubuntu | 22.04 LTS | 2.35 | Wayland, X11 | Install smoke, both arches | The oldest deb target. GTK 4.6 on the host, which the package does not use |
| Ubuntu | 24.04 LTS | 2.39 | Wayland, X11 | Install smoke, both arches | |
| Ubuntu | 26.04 LTS | 2.43 | Wayland | Install smoke, amd64, and the deb family's Wayland row | The newest deb target |
| Pop!\_OS | 22.04, 24.04 | 2.35, 2.39 | X11 (COSMIC on 24.04) | Not in CI | Supported. Ubuntu-derived, same packages, same glibc |
| Linux Mint | 22, 22.2 | 2.39 | X11, Wayland | Not in CI | Supported. Ubuntu-derived |
| Debian | 12 bookworm | 2.36 | Wayland, X11 | Install smoke, amd64 | Oldstable. Regular security support ended 12 July 2026; in LTS until 30 June 2028, which is why it stays in the matrix |
| Debian | 13 trixie | 2.41 | Wayland, X11 | Install smoke, amd64 | Stable |
| Fedora | 43, 44 | 2.42, 2.43 | Wayland | Install smoke, amd64 (44 only), and the rpm family's Wayland row | The newest rpm target. Fedora supports the current release and the one before it |
| RHEL, Alma, Rocky | 9.x | 2.34 | Wayland, X11 | Install smoke, amd64 (AlmaLinux 9) | The oldest rpm target, and the build base |
| RHEL, Alma, Rocky | 10.x | 2.39 | Wayland | Not in CI | Supported |
| openSUSE | Tumbleweed | current | Wayland, X11 | Not in CI | Best effort |
| openSUSE | Leap 15.6 | 2.31 | X11 | Not tested | **Not supported.** Below the glibc floor. The install refuses with a legible message rather than failing at exec |
| Arch, and Omarchy | rolling | 2.44 | Wayland (Hyprland), X11 | Not tested | **No package in v1.** Section 2.1 |

Architectures: **amd64** and **arm64**, both built, signed and published for both families. The
smoke runs on both arches for Ubuntu 22.04 and 24.04 and on amd64 for the rest, because what an
arm64 run proves that an amd64 run does not is that the private runtime's arm64 build links, and the
two Ubuntu rows already prove it.

X11 and Wayland are both supported and both tested; the private GTK is built with both backends.
M38 §5.3 owns the `app_id` and `StartupWMClass` pairing and does not change.

"Tested in CI" means the install smoke of section 8.4 ran on that image. "Supported best effort"
means a report against it is a bug but no gate proves it. "Not supported" means the package refuses
to install, and that refusal is a declared relation rather than a runtime check: the deb declares
`libc6 (>= 2.34)` and the rpm `libc.so.6(GLIBC_2.34)(64bit)`, so apt and dnf refuse before anything
unpacks and say what is missing.

### 2.1 Arch, and Omarchy

Omarchy is Arch with Hyprland: a Wayland-only tiling session with no GNOME session, no
`gnome-settings-daemon`, no tray, and portals from `xdg-desktop-portal-hyprland` alongside
`xdg-desktop-portal-gtk`. It is worth answering directly because it is the session where every
assumption in section 4.3 that came from a GNOME or Plasma desktop is absent.

**The runtime holds.** Arch ships glibc 2.44, far above the 2.34 floor, so the binaries run. Four
checks, each against what a Hyprland session actually has rather than what a desktop usually has.

*X11 client libraries.* The private GTK is built with both backends, and GTK 4 links the X11 client
libraries directly rather than loading them on demand, so they must be present even in a Wayland
session. They are: `hyprland` itself depends on `libx11`, `libxcb`, `libxcursor`, `libxfixes`,
`libxrender`, `libxkbcommon` and `xorg-xwayland`. The remainder, `libxi`, `libxext`, `libxrandr`,
`libxinerama` and `libxdamage`, are declared relations of the package in every family, so the
package manager brings them. A second Wayland-only build was considered and rejected: it doubles the
runtime image and the smoke matrix to save about two megabytes.

*Dark mode.* This is the one place the Omarchy answer differs from the GNOME answer, and section 4.3
is amended by it. `xdg-desktop-portal-hyprland` implements the screencast, screenshot and
global-shortcuts interfaces and **does not implement `org.freedesktop.portal.Settings`**, so the
portal path that carries `color-scheme` on GNOME, Plasma and COSMIC is not there unless
`xdg-desktop-portal-gtk` is also installed and is the one that answers. Omarchy sets its own theme by
writing `org.gnome.desktop.interface color-scheme` with `gsettings`, which means the fallback in
section 4.3 is the primary path on this session, and the bundled dconf GSettings backend is what
makes the window follow the theme instead of rendering light on a dark desktop. That turns dconf
from a fallback into a requirement, and the package declares `dconf-service` and
`gsettings-desktop-schemas` (deb) and `dconf` and `gsettings-desktop-schemas` (rpm) accordingly. The
bundled module supplies the backend; the host supplies the running `dconf-service` and the schema
that names the key.

*Icon theme.* Omarchy installs no GNOME session and may carry no `adwaita-icon-theme`. The bundled
copy at `/usr/lib/fermix-desktop/share/icons/Adwaita` is the whole reason the window has icons there,
and the host-last ordering of section 4.3 costs nothing because there is no host Adwaita to prefer.
This is the case that justifies bundling the icon theme.

*Fonts and tray.* The private fontconfig reads the host `/etc/fonts`, which Arch has, so nothing
changes. There is no tray dependency to lose: M38 §5.2 already settled that Fermix presents a window
and not a status icon, which is why the absence of a Hyprland tray is not a gap.

**Packaging is a named follow-on, not v1.** Arch needs a third format, `.pkg.tar.zst`, which the
pinned nFPM 2.47 can produce through its `archlinux` packager, plus an AUR `fermix-desktop-bin`
PKGBUILD as the channel, because there is no Fermix pacman repository and M38 §2.3 does not propose
one. That is slice 7 in section 11: scoped, small, and deliberately after the first two families
ship. Three reasons. It adds a third dependency dialect to `package_dependencies.py` and a third
image to the smoke matrix before either of the first two has shipped once. It needs an AUR
maintainer account and a person who answers for it, which M38 §2.5 currently places in the community
tier. And a rolling distribution is the one target where the private runtime's value is lowest,
because Arch's own GTK is 4.22 and far above the floor.

Until slice 7 lands, an Arch or Omarchy user is served by the standalone binary and the terminal
setup wizard, which is the existing escape hatch of M38 §2.1 and works today. The window is not
available to them, and the download page says so rather than leaving them to discover it. The
recommendation is that slice 7 is the first post-v1 slice.

## 3. Package architecture

### 3.1 Names and relations

`fermix` is the engine alone, for headless hosts. Unchanged, built in `tezra-io/fermix`, described
by M38 §2.2's first table.

`fermix-desktop` is the whole desktop product. Built here.

```
# deb control fields
Package: fermix-desktop
Provides: fermix (= <version>)
Conflicts: fermix
Replaces: fermix
Depends: libc6 (>= 2.34), libgl1, libegl1, libx11-6, libxext6, libxi6,
         libxcursor1, libxdamage1, libxfixes3, libxrandr2, libxinerama1,
         libxrender1, libxcb1, libxkbcommon0, libdbus-1-3, libz1 | zlib1g,
         libyaml-0-2, libcurl4 | libcurl4t64, libgcc-s1,
         dconf-service, gsettings-desktop-schemas
```

```
# rpm tags
Name: fermix-desktop
Provides: fermix = <version>
Conflicts: fermix
Requires: libc.so.6(GLIBC_2.34)(64bit), libGL.so.1()(64bit), libEGL.so.1()(64bit),
          libX11.so.6()(64bit), ... libdbus-1.so.3()(64bit), libz.so.1()(64bit),
          libyaml-0.so.2()(64bit), libcurl.so.4()(64bit),
          dconf, gsettings-desktop-schemas
```

Three choices in there are deliberate.

**`fermix-desktop` provides `fermix`.** Anything that depends on the engine by name, now or later,
is satisfied by the combined package. `dpkg -S /usr/bin/fermix` and `rpm -qf /usr/bin/fermix` both
answer with a package name, so `InstallMethod.dpkg_owned?` in the engine (`install_method.ex:73-84`)
keeps returning the refusal it already returns, and the rpm branch M38 §9.3 adds works the same way.
The engine cannot tell which of the two packages installed it, and does not need to.

**There is no `Obsoletes` on the rpm side.** `Obsoletes: fermix` would convert a server's engine-only
install into the desktop package on the next `dnf upgrade`, silently pulling a private GTK runtime
onto a headless host. That is the opposite of what the split exists for. `Provides` plus `Conflicts`
gives mutual exclusion without a takeover. The deb side keeps `Replaces` because dpkg needs it to
permit the file-ownership transfer of `/usr/bin/fermix` when a person deliberately swaps one package
for the other; `Replaces` alone forces nothing.

**The upgrade path from nothing.** No Linux package of either kind has ever been published, so there
is no installed base. The first published release carries both `fermix` and `fermix-desktop` at the
same version, and a person chooses one. A person who later wants the other runs
`apt install fermix-desktop` or `apt install fermix`, and apt removes the one and installs the other
in one transaction. The engine's home, its configuration and its secrets are all under `$HOME` and
`/var/lib/fermix`, none of which either package removes, so the swap keeps every setting. Section
3.4 says what happens to a running engine.

### 3.2 Installed layout

Everything below `/usr/lib/fermix-desktop/` is new. Everything else the engine owns is at the exact
path the `fermix` package puts it.

| Path | What | Owner, mode |
|---|---|---|
| `/usr/bin/fermix` | The Burrito-wrapped engine, from the engine artifact | `root:root` 0755 |
| `/usr/bin/fermix-desktop` | Symbolic link to `../lib/fermix-desktop/bin/fermix-desktop` | `root:root` 0777 link |
| `/usr/lib/fermix/cosign` | Bundled static cosign, from the engine artifact | `root:root` 0755 |
| `/usr/lib/fermix/runtime-payload/libc-musl-<digest>.so` | Verified loader input, from the engine artifact | `root:root` 0644 |
| `/usr/lib/systemd/user/fermix.service` | The vendor engine unit, from the engine artifact | `root:root` 0644 |
| `/usr/lib/systemd/user/app-io.tezra.Fermix.service` | The window's own activation unit | `root:root` 0644 |
| `/usr/share/fermix/engine.json` | Installed engine identity, from the engine artifact | `root:root` 0644 |
| `/usr/share/bash-completion/completions/fermix`, `/usr/share/zsh/site-functions/_fermix`, `/usr/share/fish/vendor_completions.d/fermix.fish` | Shell completions, from the engine artifact | `root:root` 0644 |
| `/usr/share/man/man1/fermix.1.gz` | Engine man page, from the engine artifact | `root:root` 0644 |
| `/usr/lib/fermix-desktop/bin/fermix-desktop` | The GTK4 application ELF, `RUNPATH` `$ORIGIN/../lib` | `root:root` 0755 |
| `/usr/lib/fermix-desktop/lib/*.so.*` | The private runtime: GTK4, libadwaita, GLib and everything in section 4.1's private column | `root:root` 0644 |
| `/usr/lib/fermix-desktop/lib/gdk-pixbuf-2.0/2.10.0/loaders/*.so` | Private pixbuf loaders, png, jpeg, svg, webp | `root:root` 0644 |
| `/usr/lib/fermix-desktop/lib/gdk-pixbuf-2.0/2.10.0/loaders.cache` | Generated at build, absolute paths | `root:root` 0644 |
| `/usr/lib/fermix-desktop/lib/gio/modules/libdconfsettings.so` | Private dconf GSettings backend | `root:root` 0644 |
| `/usr/lib/fermix-desktop/share/glib-2.0/schemas/gschemas.compiled` | GTK's and libadwaita's own schemas, compiled | `root:root` 0644 |
| `/usr/lib/fermix-desktop/share/icons/Adwaita/` | The bundled Adwaita icon theme, with its cache | `root:root` 0644 |
| `/usr/share/applications/io.tezra.Fermix.desktop` | Launcher, `Exec=/usr/bin/fermix-desktop %u` | `root:root` 0644 |
| `/usr/share/metainfo/io.tezra.Fermix.metainfo.xml` | AppStream metainfo | `root:root` 0644 |
| `/usr/share/dbus-1/services/io.tezra.Fermix.service` | Session D-Bus activation | `root:root` 0644 |
| `/usr/share/icons/hicolor/{16,22,24,32,48,64,128,256}x*/apps/io.tezra.Fermix.png`, `scalable/`, `symbolic/` | The application icon set | `root:root` 0644 |
| `/usr/share/fermix-desktop/build.json` | Installed GUI build identity | `root:root` 0644 |
| `/usr/share/doc/fermix-desktop/copyright` | Every bundled component, its version and its licence | `root:root` 0644 |
| `/usr/share/doc/fermix-desktop/runtime-manifest.json` | The private runtime's exact build inputs, section 4.2 | `root:root` 0644 |
| `/var/lib/fermix/runtimes/<digest>/libc-musl.so` | Materialised by postinstall, not shipped | `root:root` 0755 |

`/usr/bin/fermix-desktop` is a symbolic link rather than a shell wrapper. There is no launcher
script. Section 4.3 says why and what takes its place.

The private prefix is `/usr/lib/fermix-desktop`, not `/opt/fermix`. FHS puts internal binaries and
libraries a package does not expect anyone else to link under `/usr/lib/<package>`, and M38 §2.2
already commits to staying inside FHS and out of `/usr/local`. `/opt` would buy nothing and would
cost the `rpm` `%{_libdir}` convention.

### 3.3 Maintainer scripts

One postinstall and one postremove per package, each assembled from two fragments so that the engine
half is byte-identical to the `fermix` package's.

**postinstall**, in order: the engine fragment, taken verbatim from the engine artifact, which
materialises the musl loader into `/var/lib/fermix/runtimes/<digest>/libc-musl.so` after checking
the payload's digest against its own file name, and which is idempotent by construction (a target
already there with the right digest is left alone, and one with different contents is a refusal
rather than an overwrite); then the desktop fragment, `update-desktop-database` and
`gtk-update-icon-cache`, both guarded by `command -v` and both allowed to fail, exactly as
`packaging/scripts/postinstall.sh` does today. Neither fragment operates any user's service manager
and neither executes anything Fermix ships. The private runtime needs no configuration step at all:
its `loaders.cache`, its `gschemas.compiled` and its icon cache are generated at build time against
the fixed prefix.

**postremove**: the desktop fragment's two cache refreshes, and nothing else. The engine's postremove
is a deliberate no-op for an unchanged reason: `/var/lib/fermix/runtimes/<digest>` is
installer-managed state that outlives the package because a still-running release asks the kernel for
that exact file on every spawn.

**Purge** removes nothing further. `$XDG_STATE_HOME/fermix-desktop`, `$XDG_CONFIG_HOME/autostart`,
`$XDG_CACHE_HOME/fermix-desktop` and the engine home belong to the person, and none is
package-owned. The removal documentation names `/var/lib/fermix/runtimes` and its size, as M38 §2.2
requires.

**A running engine on upgrade.** The package manager replaces `/usr/bin/fermix` under a running
daemon. The daemon is a user-scope systemd unit that no maintainer script touches, so it keeps
running the old code from its already-mapped image until something restarts it. That is the state
M38 §9.2 calls engine skew, the app already detects it, and the copy already exists
(`src/copy.rs:1320`, `SkewNewerInstalledTitle`). Nothing about that changes. The one thing that does
change is that a running window is also replaced under itself, and the self-skew check at
`src/session/build.rs` already covers that too. Both states now arrive from one `apt upgrade`
instead of two.

### 3.4 Swapping between the two packages

`apt install fermix-desktop` on a host that has `fermix` is one dpkg transaction: `fermix` is
removed and `fermix-desktop` is unpacked. Between the two, `/usr/bin/fermix` is briefly absent and
then present again with the same bytes, because both packages carry the same engine build of the
same version. A running daemon does not notice, for the reason above. The reverse direction is the
same in the other order.

What is not attempted: preserving the daemon across the swap by arranging unit ordering. The unit is
per account and enabled by the account, no maintainer script may touch it, and a person who wants a
clean restart runs `fermix restart`. The engine's own skew notice tells them so.

### 3.5 Install experience

Ubuntu, Debian, Mint, Pop!\_OS:

```sh
sudo apt install fermix-desktop
```

Fedora, RHEL, Alma, Rocky:

```sh
sudo dnf install fermix-desktop
```

A headless host installs `fermix` instead. `packaging/INSTALL.md.tmpl` renders both lines and says
in one sentence which is which. M38 §2.6's two-package walkthrough is deleted.

## 4. The private toolkit runtime

The window links a GTK4 that the package carries. This section says exactly what that means.

### 4.1 The boundary

Two rules decide every row. A library is **private** when the host cannot be relied on to have it at
the version GTK 4.16 and libadwaita 1.6 need, or when it links a private library and would
therefore load a second copy of it. A library is **host** when it must match the running session or
the running hardware, or when its ABI is frozen and its SONAME is the same on every target in the
matrix.

The versions that decide the first rule are upstream's, read from the build files rather than
recalled. GTK 4.16 requires glib >= 2.76, cairo >= 1.18, pango >= 1.52, harfbuzz >= 2.6,
fribidi >= 1.0.6, gdk-pixbuf >= 2.30, graphene >= 1.10, epoxy >= 1.4, xkbcommon >= 0.2,
wayland-client >= 1.21, wayland-protocols >= 1.36, xrandr >= 1.2.99. libadwaita 1.6 requires
gtk4 >= 4.15.2, glib >= 2.76, fribidi, and appstream, which is a hard dependency with a subproject
fallback rather than an option. Ubuntu 22.04 ships glib 2.72, cairo 1.16, pango 1.50 and
wayland 1.20, so four of those are below the floor before anything else is considered.

| Library | Side | Why |
|---|---|---|
| GLib, GObject, GIO, GModule | **Private** | 2.72 on Ubuntu 22.04, below GTK's 2.76. Everything above it links it |
| GTK 4 | **Private** | The whole point. 4.6 on Ubuntu 22.04, 4.14 on 24.04 |
| libadwaita | **Private** | 1.1 on Ubuntu 22.04, 1.5 on 24.04 |
| pango, pangocairo, pangoft2 | **Private** | 1.50 on Ubuntu 22.04, below 1.52. Links private GLib |
| cairo, cairo-gobject | **Private** | 1.16 on Ubuntu 22.04, below 1.18 |
| harfbuzz, harfbuzz-subset | **Private** | Version is satisfied on every target, but cairo and pango both link it and a mixed private-cairo host-harfbuzz pair puts two font-shaping states in one process |
| fribidi | **Private** | Tiny, no dependencies, and bundling it removes a host ABI variable for nothing |
| graphene | **Private** | Version is satisfied, but `graphene-gobject-1.0` links GLib |
| gdk-pixbuf | **Private** | Version is satisfied, but it links GLib, and its loader cache has to name private loaders |
| librsvg | **Private** | A pixbuf loader that links private cairo, pango and GLib. The application's wordmark and most vendor marks are SVG |
| libwebp, libwebp-pixbuf-loader | **Private** | Vendor marks include WebP, and the host loader would be built against host gdk-pixbuf |
| libpng, libjpeg-turbo, libtiff | **Private** | SONAMEs diverge across the matrix: `libtiff.so.5` on Ubuntu 22.04 against `libtiff.so.6` from Debian 13 on, and `libjpeg.so.8` on Debian family against `libjpeg.so.62` on Fedora. There is no one host name to link. libtiff is built `--disable-cxx`: its `libtiffxx` wrapper links `libstdc++.so.6`, which is not on the host list below, and nothing in the runtime calls it because gdk-pixbuf's TIFF loader uses the C library. Declaring a C++ runtime as a host dependency for a wrapper no code uses would be the wrong trade |
| pixman | **Private** | cairo's rasteriser. cairo does not build without it, and a host pixman would put an undeclared ABI in the middle of text and vector rendering |
| expat | **Private** | fontconfig's XML parser. Private for the same reason fontconfig is, and taking it from the host would reintroduce the version variable that making fontconfig private removed |
| libepoxy | **Private** | Version is satisfied everywhere, but epoxy resolves GL entry points by `dlopen` of `libGL.so.1` and `libEGL.so.1` rather than by linking them, so a private epoxy still uses the host driver. Bundling costs 200 KB and removes a version variable from the GL path |
| fontconfig, freetype | **Private** | The build base is AlmaLinux 9, whose fontconfig 2.14 exports symbols Ubuntu 22.04's 2.13.1 does not, so a host-linked cairo would fail at exec on the oldest deb target. A private fontconfig reads the host `/etc/fonts` (newer fontconfig parses older configuration) and keeps its own cache under `$XDG_CACHE_HOME/fermix-desktop/fontconfig`, so the host cache version is not a variable either. The host's installed fonts are still the fonts that are found |
| wayland-client, wayland-cursor | **Private** | 1.20 on Ubuntu 22.04, below GTK's 1.21. Safe to bundle because libwayland is a wire-protocol marshaller with a frozen ABI: a newer client library speaks to an older compositor, and the host Mesa that `dlopen`s later resolves `libwayland-client.so.0` to the copy already in the process, so there is exactly one |
| wayland-protocols | **Build only** | XML definitions, compiled into GTK. Nothing ships |
| libffi, libpcre2-8 | **Private** | GLib links both. `libffi.so.6` on RHEL 9 against `libffi.so.8` on the Debian family is a SONAME split with no host answer |
| dconf | **Private** | Only `libdconfsettings.so`, the GSettings backend, built from the dconf source and installed into the private GIO module directory. It links private GLib, so a host copy cannot be loaded. The host keeps `dconf-service` and the database; the module is a client of both. Section 2.1 is why this is a requirement and not a nicety |
| appstream, libxmlb, libxml2 | **Private** | libadwaita links appstream unconditionally. appstream links GLib and libxmlb; libxmlb links GLib; libxml2's SONAME has moved on recent distributions. Built with `-Dcompose=false -Dsystemd=false -Dapt-support=false -Dstemming=false -Dgir=false -Dvapi=false`, which is the smallest configuration that satisfies the link |
| Adwaita icon theme | **Private, and host-first** | GTK 4.16 expects the symbolic icon set that ships with adwaita-icon-theme 46. Bundled at `/usr/lib/fermix-desktop/share/icons/Adwaita` and **appended** to the icon theme's search path at startup, so a host theme that matches the session still wins and the bundle is only ever a floor. Section 4.3 says why it is appended in code rather than through `XDG_DATA_DIRS` |
| **glibc, libm, libdl, libpthread, librt** | **Host** | The one thing a bundled copy cannot be. The floor is the build base's, 2.34 |
| **libgcc_s** | **Host** | Frozen ABI, present everywhere, and a second copy breaks unwinding |
| **libGL, libEGL, libgbm, libGLdispatch, Vulkan ICDs, libdrm** | **Host** | They are the driver. A bundled Mesa cannot talk to the kernel module the host has |
| **libdbus-1** | **Host** | It talks to the session bus and the ABI has been frozen since 1.0. Portals, the D-Bus activation file and the accessibility bus all go through it, and all three are session state |
| **libX11, libxcb, libXext, libXi, libXcursor, libXdamage, libXfixes, libXrandr, libXinerama, libXrender** | **Host** | Wire-protocol clients with frozen ABIs, and the host GL stack links the same ones. Two copies of Xlib in one process is a known way to lose the XCB lock |
| **libxkbcommon** | **Host** | GTK needs >= 0.2 and the oldest target has 1.0.3. It consumes the keymap the compositor sends, so it is session-adjacent, and a second copy buys nothing |
| **zlib, libyaml, libcurl, libzstd, liblzma** | **Host** | Frozen SONAMEs on every target: `libz.so.1`, `libyaml-0.so.2`, `libcurl.so.4`, `libzstd.so.1`, `liblzma.so.5` |
| **libselinux, libmount, libblkid** | **Neither** | GLib is built with `-Dselinux=disabled -Dlibmount=disabled`. The features they enable, SELinux context propagation and `GUnixMountMonitor`, are not reachable from this application, and disabling them removes three host dependencies |

### 4.2 Building it

The runtime is built by `packaging/runtime/build_runtime.sh` from a lock file,
`packaging/runtime/RUNTIME.lock.json`. One entry per component: `name`, `version`, `url`, `sha256`,
`build_system` (`meson`, `autotools`, `cmake` or `cargo`; libjpeg-turbo 3.2.0 is the cmake one),
`options` (the exact configure flags), and
`patches` (a list of file names under `packaging/runtime/patches/`, normally empty). The script
refuses a tarball whose digest is not the locked one, and refuses to run at all if the lock file
names a component that is not in its build order.

The base image is `packaging/docker/Dockerfile.runtime`, `FROM almalinux:9` pinned by digest. It
carries the host-side development headers from the table above, a pinned Meson and Ninja, a pinned
Rust toolchain for librsvg, and nothing else. AlmaLinux 9 is the base because glibc 2.34 is the
lowest floor that covers RHEL 9 and Ubuntu 22.04 in one build, and because building against the
oldest host headers in the matrix is what makes the host-provided half forward compatible.

**Every component is configured with `--prefix=/usr/lib/fermix-desktop`, which is the final
installed path**, and is built and installed at that path in the image. Nothing is built under a
staging prefix and relocated afterwards. This is not a convenience. A GLib configured with that
prefix compiles that prefix in as its GIO module directory, and a gdk-pixbuf configured with it
compiles in the private `loaders.cache` path, so both are correct with no environment variable at
all. Relocating a tree built under `/opt/runtime` would leave both pointing at a directory that does
not exist on a user's machine, and the usual repair for that is exactly the two environment
variables this design does not want. The one cost is that the build image cannot hold two runtimes
at once, which nothing needs.

**The boundary of section 4.1 is enforced by the build, not by review.** After each component is
installed, `build_runtime.sh` reads every `NEEDED` entry of every object it just produced and fails
if one resolves outside the private prefix to a library that is not on the host list. That check is
what caught libtiff's `libtiffxx` pulling in `libstdc++.so.6`, a host dependency nobody had declared
and no code in the runtime calls, and it caught it during a build rather than during an install on
somebody's Ubuntu 22.04. A boundary this long is not something a person re-reads correctly on the
twentieth component, so it is a gate.

**Caching.** The runtime is not rebuilt per run. `build_runtime.sh` computes a cache key from the
sha256 of `RUNTIME.lock.json`, the patch directory and the Dockerfile, and publishes the result as
an OCI image, `ghcr.io/tezra-io/fermix-desktop-runtime:<key>-<arch>`, whose entire content is
`/usr/lib/fermix-desktop`. Every application build pulls that image and copies that directory out of
it to the same absolute path. The runtime is rebuilt only when the key changes, which is when a
component is bumped or the base image is bumped, so the ordinary release run pays a pull and not a
compile. A run whose key has no published image builds it and pushes it, once, in a job of its own
that every architecture's build job waits on.

**Reproducibility.** Sources are pinned by digest, the compiler is pinned by the base image digest,
`SOURCE_DATE_EPOCH` is set from the lock file's own commit, and the private prefix is a fixed
absolute path, so no build path leaks into the output. `build_runtime.sh --verify` rebuilds and
compares the tree digest against `runtime-manifest.json`. It is a gate in `packages.yml`, not in
every push, because it costs a full compile.

`runtime-manifest.json` is the lock file plus, for each component, the sha256 of every installed
file. It ships at `/usr/share/doc/fermix-desktop/runtime-manifest.json`, which is what makes the
licensing obligation of section 4.6 answerable from an installed machine.

### 4.3 Runtime wiring

**Library resolution is `RUNPATH`, not `LD_LIBRARY_PATH`.** Every private object is linked with
`-Wl,-rpath,$ORIGIN/../lib -Wl,--enable-new-dtags`, the application ELF included. There is no
launcher script and no environment variable naming a library directory. `LD_LIBRARY_PATH` is
inherited by every child, `/usr/bin/fermix` and `xdg-open` among them, and scrubbing it in every
spawn path is a standing hazard; `RUNPATH` is per object, so the host Mesa that `dlopen`s later is
unaffected; and a wrapper script means `/usr/bin/fermix-desktop` is not the process the desktop
sees, which complicates `StartupWMClass`, D-Bus activation and unit accounting for no gain.
`/usr/bin/fermix-desktop` is therefore a symbolic link to the real ELF, whose `$ORIGIN` resolves
through the link target as specified.

**Exactly one environment variable is set, and it is set by the application itself**, at the top of
`main`, before GTK is initialised and before any thread exists.

`GIO_MODULE_DIR` and `GDK_PIXBUF_MODULE_FILE` are **not set at all**. Both libraries were configured
with the installed prefix, so their compiled-in defaults already name
`/usr/lib/fermix-desktop/lib/gio/modules` and
`/usr/lib/fermix-desktop/lib/gdk-pixbuf-2.0/2.10.0/loaders.cache`. Setting either would restate a
fact the library already holds and would then have to be unset for every child.

`XDG_DATA_DIRS` is **not mutated**. The bundled Adwaita theme is registered with
`gtk::IconTheme::add_search_path` on the default icon theme at startup, which appends, so a host
Adwaita found through the unmodified `XDG_DATA_DIRS` is still preferred and the bundle is the floor.
That keeps the host-first decision of section 2.1 and costs one line of Rust instead of an
inherited variable.

| Variable | Set to | Why |
|---|---|---|
| `GSETTINGS_SCHEMA_DIR` | `/usr/lib/fermix-desktop/share/glib-2.0/schemas` | GLib reads it from the environment, there is no compiled-in equivalent, and it **prepends** to the search path rather than replacing it |

Private-first is deliberate here, and it is the one place in this design where the private copy must
win. GLib resolves a schema id to the first source that has it, and `g_settings_new` on a schema
that exists but lacks the key being read is a hard abort, not a fallback. A host GTK 4.6 installs
`org.gtk.gtk4.Settings.FileChooser` without keys that GTK 4.16 reads, so a host-first order would
abort the process on Ubuntu 22.04. Prepending the private directory makes the private compiled
schemas answer for GTK's and libadwaita's own ids, while every id they do not define, including
`org.gnome.desktop.interface`, still resolves through the untouched `XDG_DATA_DIRS`. That is the
whole reason the order is this way round and not the other.

`RuntimeEnv::capture()` records that variable's prior value, including its absence, and is the
single source every child launch uses. It is a plain Rust struct with a unit test asserting that a
spawned child's environment equals the process's entry environment, which is a better gate than a
shell wrapper's careful `unset` lines because it fails in `cargo test` rather than on a user's
machine. `ServiceRunner` (`src/service/runner.rs`) and every URI launch take it.

**An uninstalled build.** The runtime lives at `/usr/lib/fermix-desktop` inside the build container
too, at the same absolute path, so a developer build needs no relocation and no special case: the
crate finds the toolkit through `PKG_CONFIG_PATH=/usr/lib/fermix-desktop/lib/pkgconfig`, and the
binary it produces in `target/debug` resolves its libraries through the same compiled-in prefix that
the installed binary uses. The only difference between a development run and an installed run is
that `$ORIGIN/../lib` does not apply to a binary in `target/debug`, so `container_build.sh` and
`capture.sh` export `LD_LIBRARY_PATH=/usr/lib/fermix-desktop/lib` for that one case. That is a
property of the container, not of the product, and `check_private_runtime.sh` asserts the installed
ELF needs no such variable.

**GIO modules.** A private GLib cannot load a host GIO module, so three things that a stock GTK
application gets for free have to be decided rather than inherited.

*dconf.* Bundled, as `libdconfsettings.so` in the private GIO module directory. Without it
`GSettings` falls back to the memory backend and every host setting reads as its default. dconf
talks to `dconf-service` over the session bus and memory-maps `~/.config/dconf/user`, both safe
across a GLib version boundary. The bundled module is the backend; the host supplies the running
`dconf-service` and the schemas, which is why both are declared relations.

*glib-networking.* **Not bundled.** The application opens no network connection: everything it shows
goes over `daemon.sock` and every outbound request belongs to the engine, a separate process with
its own TLS. `scripts/check_no_network.sh` (new) fails the build if the application ELF names any
symbol from `gio`'s TLS or resolver surface, so the claim is a gate rather than a comment.

*gvfs.* Not bundled. The file chooser goes through `org.freedesktop.portal.FileChooser`, which is
D-Bus and needs no GIO module, and is already how the diagnostics export works on a sandboxed
desktop.

**Launching a URI, and the environment the browser gets.** This is the one leak a `RUNPATH` design
does not close by itself, and it needs naming rather than assuming. `gtk::UriLauncher` uses the
portal only inside a sandbox; outside one it calls `g_app_info_launch_default_for_uri`, which forks
the user's browser as a child of this process, and that child inherits `GSETTINGS_SCHEMA_DIR`. A
Firefox or a Nautilus that starts with a private GTK 4.16 schema directory prepended is a real
failure, not a theoretical one.

**The decision is to launch through `gio::AppInfo` with an explicit `gio::AppLaunchContext`**, not
to call the OpenURI portal directly. `RuntimeEnv` writes its recorded prior state onto that context
with `setenv` and `unsetenv`, so the child gets the environment this process started with, and the
portal is left to the cases GTK already routes to it. The portal was the alternative and is refused
for two reasons: it is absent or partial on some sessions in the matrix, section 2.1 being the
example, so it would need this fallback anyway; and it would put the app's only direct D-Bus
client code in a path that has a working library call.

`FileLauncher::open_containing_folder` needs no such care and is left alone: it calls
`org.freedesktop.FileManager1` over D-Bus, the file manager is activated by the bus rather than
forked from this process, and it inherits nothing from here at all.

The test is in `tests/runtime_env.rs`: build the launch context `RuntimeEnv` produces, read back the
environment it would hand a child, and assert it is equal to the process's entry environment
captured before `main` set anything. It fails in `cargo test`, with no display and no browser.

**Dark mode and settings.** GTK 4.16 reads `color-scheme`, the interface font and the cursor theme
from `org.freedesktop.portal.Settings` over D-Bus wherever a backend implements that interface,
which is GNOME, Plasma and COSMIC, and that path does not touch dconf at all. It is not universal:
`xdg-desktop-portal-hyprland` implements no Settings interface, so on a Hyprland session the
bundled dconf backend is not the fallback but the only path, and section 2.1 owns that case. A plain
X11 session on Ubuntu 22.04 reads the host's settings the same way.

**Input methods. The package ships no input modules and has no `immodules` directory.** GTK 4 is not
GTK 3 here, and an earlier draft of this section described the GTK 3 arrangement. GTK 4.16's
`modules/` builds exactly two things, `printbackends` and `media`, and no input module at all; its
`GtkIMContext` implementations are registered on a GIO extension point, and the ones that matter,
`simple`, `none` and `wayland`, are compiled into libgtk. The loading mechanism still exists,
`gtk_im_modules_init` scans an `immodules` directory for third-party modules, but nothing Fermix
builds puts a file there, and a host fcitx5 or ibus module could not be loaded into it anyway
because it links host GTK and host GLib. Shipping the empty directory would be a claim the package
cannot keep, so it is not in the layout table.

What actually serves input methods is unchanged by that, and is the reason this costs nothing. On
Wayland, `text-input-v3` puts input method handling in the compositor, and fcitx5 and ibus both work
with no module in any application. On X11, GTK's ibus context talks to the ibus daemon over D-Bus
rather than through a module, and an fcitx5 user falls back to XIM, which works and is worse. The
acceptance runbook gains one row for a CJK input method on each of Wayland and X11, which is where
that last sentence gets checked rather than believed.

**Fonts.** The private fontconfig reads the host `/etc/fonts/fonts.conf` and therefore the host's
installed fonts, its aliases and its hinting settings. No font is bundled. The private cache lives
under `$XDG_CACHE_HOME/fermix-desktop/fontconfig` so the host cache's format version is not shared,
and the first launch pays a font scan of a second or two, once.

**Accessibility.** GTK 4's AT-SPI bridge is compiled into GTK and speaks D-Bus to the host
`at-spi2-registryd`. Host `libdbus-1` is what carries it, which is one of the reasons dbus is on the
host side. Orca reads the window without anything further.

**Locale.** `gettext` is glibc's, catalogues install to `/usr/share/locale` as they do today, and the
crate's `gettext-system` feature is unchanged.

### 4.4 The crate's feature floor

The crate pins `gtk4 = { version = "0.11.4", features = ["v4_16"] }` and
`libadwaita = { version = "0.9.2", features = ["v1_6"] }` (`App/Fermix/Cargo.toml:27-28`).

**The floor stays at 4.16 and 1.6, and the bundled runtime is exactly GTK 4.16.x and
libadwaita 1.6.x.** Compiling against the version that ships is the whole value of the feature
floor: a symbol above the floor is a compile error rather than a missing symbol on a user's machine,
and that comment in `Cargo.toml` stays true for a new reason. Raising the floor would mean bundling
a newer GTK, which costs a larger security surface for features the application does not use.

The bundled point releases are whatever the lock file names, and bumping them within 4.16.x and
1.6.x needs no crate change. Moving to 4.18 would be a deliberate change to `Cargo.toml`, the lock
file and this section together, and section 4.6 says when that happens.

### 4.5 Security maintenance

Bundling means owning the CVE surface of twenty-odd libraries that a distribution would otherwise
patch. That is the real cost of this amendment and it is answered by a rail, not by intent.

`.github/workflows/runtime-watch.yml` (new) runs weekly and on demand. For every component in
`RUNTIME.lock.json` it reads the upstream release feed and the OSV database for that package and
version, and opens or updates one issue titled `runtime: <component> <installed> -> <available>`
carrying the advisory identifiers. It opens nothing when there is nothing to say. The issue is
assigned to the release owner. There is no automatic bump, because a GTK point release can change
rendering and the captures are a reviewed artifact.

A toolkit-only rebuild is a desktop-only release. Section 6.4 says how it is versioned.

### 4.6 Licensing

GTK, GLib, libadwaita, pango, gdk-pixbuf, libxmlb and appstream are LGPL 2.1 or LGPL 2.1-or-later.
dconf is LGPL 2.1-or-later, and only its GSettings backend module ships. cairo is LGPL 2.1 or
MPL 1.1. librsvg is LGPL 2.1-or-later. harfbuzz, graphene, fribidi, libepoxy,
wayland, fontconfig, libpng, libjpeg-turbo, libtiff, libwebp, pixman and expat are permissive.
freetype is under the FTL or GPL 2. The application is MIT.

The list above is prose and will drift. `packaging/copyright` is generated from
`RUNTIME.lock.json` and `scripts/check_copyright.sh` fails when the two disagree, so the installed
copyright file is the authority and this paragraph is an orientation.

Three obligations, three answers.

**Relinking and replacement.** LGPL 2.1 §6 is satisfied by dynamic linking: every private library is
a shared object in `/usr/lib/fermix-desktop/lib/` that a person can replace with their own build of
the same SONAME, and the application will load it. The `RUNPATH` is `$ORIGIN/../lib`, so a
replacement in that directory takes effect with no relinking and no environment variable. The
copyright file says so in one sentence.

**Source.** Every component's exact source tarball URL and sha256 is in
`RUNTIME.lock.json` and in the installed `runtime-manifest.json`, and every patch is in
`packaging/runtime/patches/`. In addition, each release publishes
`fermix_desktop_runtime_sources_<version>.tar.gz`, containing every locked tarball, every patch and
`build_runtime.sh`, as a release asset with the same cosign sidecars as the packages. That is a
written offer honoured by a link rather than by post.

**Copyright file.** `packaging/copyright` is generated from `RUNTIME.lock.json` rather than
maintained by hand, in Debian machine-readable format, one stanza per component with its version,
its licence identifier and its full licence text. `scripts/check_copyright.sh` (new) fails if the
generated file and the lock file disagree, which is what stops a component being added without its
licence.

## 5. Engine handoff

### 5.1 The artifact

`fermix_app_engine_linux_x86_64.tar.gz` and `fermix_app_engine_linux_aarch64.tar.gz`, published as
assets of every engine release, each with `.sha256`, `.sig` and `.pem` beside it, cosign keyless,
signed in the engine's existing `sign-candidate` job alongside the macOS app-engine tarballs.

The archive root is `fermix_app_engine/`, as on macOS. Inside:

```
fermix_app_engine/
  engine-manifest.json
  tree/usr/bin/fermix
  tree/usr/lib/fermix/cosign
  tree/usr/lib/fermix/runtime-payload/libc-musl-<digest>.so
  tree/usr/lib/systemd/user/fermix.service
  tree/usr/share/fermix/engine.json
  tree/usr/share/bash-completion/completions/fermix
  tree/usr/share/zsh/site-functions/_fermix
  tree/usr/share/fish/vendor_completions.d/fermix.fish
  tree/usr/share/man/man1/fermix.1.gz
  tree/usr/share/doc/fermix/copyright
  maintainer/postinstall.sh
  maintainer/postremove.sh
  nfpm-contents.yaml
```

`tree/` is the staged tree the `fermix` deb is built from, produced by the same function in
`scripts/release/linux_packages.py` that produces the deb's stage. That is the mechanism that makes
"the engine layout is identical in both packages" true by construction rather than by review.
`maintainer/` holds the same two scripts `packaging/linux/scripts/` holds. `nfpm-contents.yaml` is
the `contents:` block of `packaging/linux/nfpm-fermix.yaml.tmpl` with `{{STAGE}}` already resolved,
so the desktop build concatenates it into its own configuration instead of restating every path and
mode.

`/usr/share/doc/fermix/changelog.Debian.gz` is deliberately absent from the tree: the combined
package ships one changelog, its own, at `/usr/share/doc/fermix-desktop/`.

### 5.2 The manifest

`engine-manifest.json`, schema version 1, validated by the same `package_app_engine.py` code path the
macOS artifact uses, with one new distribution identity.

```json
{
  "schema_version": 1,
  "identity": {
    "engine_id": "fermix-core",
    "product_version": "0.11.0",
    "build_id": "release-1234567890",
    "source_commit": "<40 hex>",
    "distribution_identity": "linux_package",
    "artifact_target": "linux_x86_64",
    "architecture": "x86_64"
  },
  "protocols": {
    "management": { "current_version": 1, "minimum_version": 1, "maximum_version": 1 },
    "realtime":   { "current_version": 1, "minimum_version": 1, "maximum_version": 1 }
  },
  "provenance": {
    "oidc_issuer": "https://token.actions.githubusercontent.com",
    "certificate_identity":
      "https://github.com/tezra-io/fermix/.github/workflows/release.yml@refs/tags/v0.11.0"
  },
  "tree_sha256": "<sha256 of the canonical tree digest over tree/ and maintainer/>",
  "inventory": [ { "path": "usr/bin/fermix", "kind": "file", "mode": 493, "sha256": "..." } ]
}
```

`distribution_identity` is `linux_package`, the same value the `fermix` deb's engine carries, because
it is the same build. `_validate_identity` in `scripts/release/package_app_engine.py:200` currently
requires `macos_app`; it takes a per-target expected value. The inventory validator's Mach-O
architecture check is replaced by an ELF check for the Linux targets, and the check that matters is
the one that already exists for macOS: every ELF interpreter in the tree names
`/var/lib/fermix/runtimes/<digest>/libc-musl.so` and that digest is the payload the tree carries.

### 5.3 Changes in the fermix repository

| File | Change |
|---|---|
| `scripts/release/linux_packages.py` | Extract the staging into `stage_engine_tree(target, version, out_dir)`, called by both the nfpm path and the new archive path. Emit the resolved `nfpm-contents.yaml` |
| `scripts/release/build_app_engine.sh` | Accept `linux_x86_64` and `linux_aarch64`, and for those targets call `stage_engine_tree` rather than `mix release fermix_app_engine`. Add `--container` and `--dev`, section 8.5. The macOS path is untouched |
| `scripts/release/package_app_engine.py` | Per-target `distribution_identity`; ELF inventory validation; the loader-digest cross-check |
| `scripts/release/verify_app_engine.py`, `verify_app_engine.sh` | Accept the Linux targets |
| `scripts/release/test_package_app_engine.py`, `test_build_app_engine.py`, `test_verify_app_engine.py` | Fixtures for a Linux tree |
| `.github/workflows/release.yml` | `app-engine` matrix gains `{ os: ubuntu-24.04, target: linux_x86_64 }` and `{ os: ubuntu-24.04-arm, target: linux_aarch64 }`, reusing the `linux-packages` job's toolchain steps. `sign-candidate`, `verify-candidate`, `stage-release` and `verify-published` already glob `fermix_app_engine_*` and need no change. A new `dispatch-linux-desktop` job, `needs: promote`, section 6 |
| `docs/design/MILESTONE_38_ENGINE_HANDOFF.md` | A section naming the new asset and its manifest |

**The engine-only deb and rpm rail is unchanged.** `linux-packages` still builds
`fermix_<version>_<arch>.deb` and `fermix-<version>-1.<arch>.rpm`, still publishes them, and they
are still the headless product. The new job adds an asset; it removes nothing.

### 5.4 The pin, and the scripts around it

`engine/PIN.json` becomes schema version 2. It names the artifact, not the packages.

```json
{
  "schema_version": 2,
  "repository": "tezra-io/fermix",
  "certificate_oidc_issuer": "https://token.actions.githubusercontent.com",
  "tag": "v0.11.0",
  "engine_version": "0.11.0",
  "source_commit": "<40 hex>",
  "certificate_identity":
    "https://github.com/tezra-io/fermix/.github/workflows/release.yml@refs/tags/v0.11.0",
  "artifacts": {
    "linux_x86_64": { "asset": "fermix_app_engine_linux_x86_64.tar.gz", "sha256": "..." },
    "linux_aarch64": { "asset": "fermix_app_engine_linux_aarch64.tar.gz", "sha256": "..." }
  }
}
```

The all-or-nothing rule survives: `scripts/engine_pin.sh` still refuses a half-filled pin, and the
dangerous state it names is still the dangerous state. What changes is that an unpinned pin is no
longer buildable at all. Today a build with no pin produces a package that declares a dependency on
an engine that does not exist; from this amendment on, no pin means no engine, so
`build_packages.sh` refuses.

`scripts/fetch_engine.sh` keeps its shape and downloads two archives with their three sidecars each
instead of four packages. `scripts/verify_engine.sh` keeps both of its checks, the pin's sha256 and
the cosign identity, and gains a third: after verifying, it unpacks into a staging directory and
checks `engine-manifest.json` against the pin, that `identity.source_commit` is the pinned commit,
that `identity.product_version` is `engine_version`, that `provenance.certificate_identity` is the
pinned identity, and that `tree_sha256` is the digest of what was actually unpacked. The unpack is
in `verify_engine.sh` rather than a new script because the rule that a file has no standing until it
is verified is easier to hold when the only thing that writes the staging tree is the verifier.

`scripts/verify_engine_test.sh` keeps its offline fixture discipline and grows cases for each new
refusal.

## 6. Release automation

### 6.1 The sequence

1. A person pushes `vX.Y.Z` to `tezra-io/fermix`.
2. `release.yml` runs as it does today: `preflight`, `standalone`, `linux-packages`, `app-engine`
   (now four targets), `sign-candidate`, `verify-candidate`, `stage-release`, `verify-published`,
   `promote`.
3. A new job `dispatch-linux-desktop`, `needs: promote`, mints a short-lived installation token and
   sends a `repository_dispatch` to `tezra-io/fermix-linux` with `event_type: engine-released` and a
   payload of `{ tag, version, source_commit, certificate_identity, artifacts }`, where each artifact
   entry carries the asset name and the digest read from the release's own `.sha256` sidecar. It runs
   after `promote` so the assets are certainly published; a dispatch for a release that is still
   staged would name assets nothing can download.
4. `engine-release.yml` in `tezra-io/fermix-linux` receives it. It writes `engine/PIN.json` from the
   payload, sets `App/Fermix/Cargo.toml`'s version and
   `packaging/io.tezra.Fermix.metainfo.xml`'s release entry to the engine version, runs
   `scripts/engine_pin.sh`, `scripts/fetch_engine.sh` and `scripts/verify_engine.sh` so that a pin it
   cannot verify never becomes a commit, and opens a pull request from `engine/vX.Y.Z` with
   auto-merge enabled. It pushes the branch and opens the pull request with an **App installation
   token**, not with `GITHUB_TOKEN`.
5. The pull request's required checks are the full `app.yml` and `packages.yml` gate sets, which
   include a real package build and the install smoke matrix. Auto-merge lands it when they pass.
   Nothing merges on a red check, and a failed check leaves an open pull request with the failure on
   it, which is the retry surface.
6. A `tag` job on push to `main`, guarded to commits whose message is the bot's bump line and whose
   version is not already tagged, pushes `fermix-desktop-vX.Y.Z`, again with an App installation
   token.
7. `release-fermix-desktop.yml` runs on that tag: `version`, `runtime` (pull or build the runtime
   image), `build` per architecture, `engine` (fetch and verify), `sign`, `verify` (the smoke
   matrix), `publish`.
8. `publish` runs in the protected `release-linux` environment and waits for its required reviewer.

**Why steps 4 and 6 cannot use `GITHUB_TOKEN`, and what that costs.** GitHub does not start a
workflow run from an event produced by the default token. A pull request opened with `GITHUB_TOKEN`
fires no `pull_request` workflows, so the bump would sit with no checks and auto-merge would never
have anything to wait on; a tag pushed with `GITHUB_TOKEN` fires no `push` tag workflows, so
`release-fermix-desktop.yml` would never run. Both are silent: the chain stops with everything
green and nothing published. The App installation token is a different actor and does start runs.
`workflow_call` was the alternative and is refused because it would collapse the bump and the
release into one run, losing the pull request that is the review surface and the retry surface of
section 6.3.

The cost is that the App's credentials are secrets in `tezra-io/fermix-linux` as well as in
`tezra-io/fermix`. **The property that the engine repository cannot merge anything is preserved by
branch protection, not by token scope.** `main` requires the full check set and does not allow a
push that bypasses it, and the App has no Administration permission, so it cannot relax that. The
worst an actor holding the App's key can do is open a pull request and wait for the same gates
everyone else waits for, and then be stopped again at the `release-linux` environment.

A person is involved at step 1 and step 8 and nowhere else. No human edits a pin.

### 6.2 Credentials

| Secret | Where | Scope |
|---|---|---|
| `FERMIX_RELEASE_APP_ID`, `FERMIX_RELEASE_APP_PRIVATE_KEY` | `tezra-io/fermix`, repository secrets | One GitHub App, Fermix Release Bot, installed on `tezra-io/fermix-linux` only, with **Contents: write** and **Pull requests: write**. No Actions, no Administration, no organisation permission. Contents write is what `repository_dispatch` requires. The job mints a token with `actions/create-github-app-token` and it expires in an hour |
| The same two secrets | `tezra-io/fermix-linux`, repository secrets | The same App. `engine-release.yml` mints its own token to push the bump branch, open the pull request and enable auto-merge; the `tag` job mints one to push the tag. Required because `GITHUB_TOKEN` starts no workflow run, section 6.1 |
| `GITHUB_TOKEN` in `engine-release.yml` | `tezra-io/fermix-linux` | Read only, for the checkout. It writes nothing |
| OIDC | both repositories | `id-token: write` on the signing jobs only, as today. No cosign key exists to leak |
| `GHCR_TOKEN` | `tezra-io/fermix-linux` | `packages: write` on the runtime image only, for the runtime cache. `packages: read` everywhere else |

Neither repository holds a credential that can write to `fermix-linux`'s workflows, its branch
protection or its environments, so a compromise of the dispatch path can open a pull request and
cannot merge one.

### 6.3 Failure handling

A failed desktop release is an open pull request or a red tag run, and both are visible without
looking for them. `engine-release.yml` posts the failure to the release owner. Three shapes:

*The dispatch never arrives.* The engine release is fine and no desktop release exists. Re-running
`dispatch-linux-desktop` from the engine's Actions tab is the whole retry, and it is idempotent
because the pull request branch name is derived from the tag.

*The pull request's checks fail.* This is the case that matters, because it means the new engine and
the current application disagree about something. Nothing is published. A person fixes it on the
branch and the auto-merge proceeds. The engine release stays published; there is no rollback of the
engine because of a desktop failure.

*The tag run fails after the tag exists.* Re-running the failed job is the first move. If the tag
itself is wrong, it is deleted and re-pushed, which is safe because `publish` has not run and no
asset exists. `scripts/refuse_published_release.sh` in the engine repository is the precedent for
refusing to re-cut a published one, and `release-fermix-desktop.yml`'s `publish` job gains the same
refusal.

### 6.4 Versioning

**The desktop version is the engine version.** `fermix-desktop 0.11.0` carries engine 0.11.0. That
is enforced three ways, unchanged from today: the tag, `Cargo.toml` and the pin must agree, and
`build_packages.sh` refuses otherwise (`scripts/build_packages.sh:135-147`).

**A desktop-only fix is `<engine version>+<n>`.** `0.11.0+1` for the first, `0.11.0+2` for the
second. A toolkit CVE rebuild, a copy fix, a packaging fix, all of them.

This replaces M38 §2.2's rule that a packaging fix is published as the next version. That rule
existed for exactly one reason, that `Depends: fermix (= <version>)` and `Requires: fermix = <version>`
had to keep meaning the same thing, and it cost a version number for every packaging typo. With no
exact-version relation between two packages, the reason is gone.

`+` is chosen over a Debian revision because nFPM's `release` field is a single top-level value that
feeds the Debian revision and the RPM `Release` tag together, which is the asymmetry
`docs/RELEASING.md` already documents. `+` lives in the upstream version on both sides, so `release`
stays empty, the rpm keeps its `-1`, and both `dpkg --compare-versions` and `rpmdev-vercmp` order
`0.11.0 < 0.11.0+1 < 0.11.1`. The refusal on `-` and `:` stays: no Debian revision, no rpm epoch,
and a prerelease tag still produces no packages.

`build_packages.sh` accepts `X.Y.Z` and `X.Y.Z+N`, and when the version carries a `+N` it requires
the pin's `engine_version` to be the `X.Y.Z` part rather than the whole string.

**`+` is correct in both package families and is not yet proven anywhere else, so it is proven
before it is relied on.** Six places handle the character and each is verified by slice 6 rather
than assumed:

| Where | What must hold |
|---|---|
| Git tag ref | `fermix-desktop-v0.11.0+1` is a legal ref. `+` is not in git's forbidden set |
| nFPM output file names | The built files are `fermix-desktop_0.11.0+1_amd64.deb` and `fermix-desktop-0.11.0+1-1.x86_64.rpm`. Whether nFPM sanitises `+` out of the name it chooses, rather than out of the version field, is read from the run |
| GitHub release asset names | GitHub replaces some characters in uploaded asset names. If `+` becomes `.` the published name and the name `fetch_engine.sh` asks for diverge |
| `gh release download --pattern` | The pattern is matched against the published name, so it must be given whatever the previous row produced, URL encoding included |
| Cargo | `version = "0.11.0+1"` is valid semver build metadata, and **cargo ignores build metadata when comparing versions**. Nothing in this repository compares crate versions, and `build_packages.sh` compares the strings itself, so this is a non-issue that is recorded so nobody adds a comparison later |
| AppStream | `appstreamcli validate` must accept the metainfo `<release version="0.11.0+1">`, and the version ordering a software centre derives from it must put it after `0.11.0` |

Slice 6's acceptance is a dry-run `0.11.0+1` release that goes end to end: built, signed, uploaded
to a draft release, downloaded again by name, and installed by the smoke on one deb and one rpm
image, with `dpkg --compare-versions 0.11.0 lt 0.11.0+1` and the rpm equivalent asserted.

**The fallback, if GitHub rewrites `+` in asset names.** The version keeps its `+`, because that is
what dpkg and rpm order correctly and it is the version a user sees; only the asset file name
changes. `build_packages.sh` gains one rule, that the asset name substitutes `+` with `~plus~`, and
`fetch_engine.sh` and the publish job share one function that performs the same substitution, so the
name asked for is the name published. Nothing about the installed package changes. This is written
down now so that the dry run has a defined outcome either way rather than becoming a decision under
release pressure.

## 7. Application code changes

Small, as expected. Ten places.

| File | Line | Today | Change |
|---|---|---|---|
| `App/Fermix/src/paths.rs` | 16 | `pub const PACKAGED_CLI: &str = "/usr/bin/fermix";` | **No change.** The path is the same in both packages. The doc comment's reason changes from "the `fermix` package owns it and we declare an exact-version dependency" to "this package owns it" |
| `App/Fermix/src/runtime.rs` | new | | `RuntimeEnv::capture()`, the single `GSETTINGS_SCHEMA_DIR` set of section 4.3, the `IconTheme::add_search_path` call, and `launch_context()` which builds a `gio::AppLaunchContext` with the entry environment restored. About 90 lines with its tests |
| `App/Fermix/src/main.rs` | | `fn main()` initialises the application | Call `runtime::RuntimeEnv::capture()` as the first statement and pass it into the application |
| `App/Fermix/src/service/runner.rs` | | Spawns `Paths::cli()` through GIO | Take a `RuntimeEnv` and apply it to the child environment |
| `App/Fermix/src/session/desktop.rs` | | Launches URIs and opens the log folder | URI launching moves from `gtk::UriLauncher` to `gio::AppInfo` with `RuntimeEnv::launch_context()`, section 4.3. `open_containing_folder` is unchanged, because `org.freedesktop.FileManager1` inherits nothing |
| `App/Fermix/tests/runtime_env.rs` | new | | The child-environment equality test of section 4.3 |
| `App/Fermix/src/copy.rs` | 1146 | `RecoveryPackageRepair`: "Reinstall the packages with your package manager, then try again." | "Reinstall Fermix with your package manager, then try again." |
| `App/Fermix/src/copy.rs` | 1256 | `PreflightPreManagementDaemonBody`: "... Update the packages, then start again." | "... Update Fermix, then start again." |
| `App/Fermix/src/copy.rs` | 1295 | `ActivationProtocolMismatchBody`: "... Update both packages together." | "... Update Fermix and restart the background service." The two halves can still disagree, because an upgrade replaces files under a running daemon; what can no longer happen is two packages at different versions |
| `App/Fermix/tests/packaging.rs` | 29-44 | The claimed install list | Add `/usr/lib/fermix-desktop/bin/fermix-desktop`, the private prefix and the engine's paths; `/usr/bin/fermix-desktop` becomes a link assertion |
| `App/Fermix/tests/packaging.rs` | 376-393 | `the_template_declares_the_toolkit_floor_and_the_exact_engine_relation` | Rewritten as `the_template_declares_the_glibc_floor_and_the_engine_alternative`: assert `Provides`, `Conflicts`, `Replaces` in both spellings and the absence of any `libgtk-4-1` or `gtk4` relation |

**Nothing about engine and GUI version skew changes.** `src/session/build.rs` compares a compiled
build id against `/usr/share/fermix-desktop/build.json`, which is a statement about this process
against the installed application, and that is still exactly right: the package manager still
replaces files under a running window. `Key::SkewStaleGuiTitle` and `Key::SkewNewerInstalledTitle`
(`src/copy.rs:1320`, `:1337`) and `Key::PreflightEngineSkewTitle` (`:1259`) all stay, and their text
already avoids naming a package.

**No host toolkit check exists to remove.** The application never checks a GTK version at runtime;
the floor was a packaging relation and a compile-time feature, and only the first of those goes.

**Captures. The whole set is re-taken, and the copy edits are not the reason.** These are two
independent facts and an earlier draft of this section ran them together, naming three captures and
attributing them to the wording. Both halves are corrected here.

*The reason the batch exists is that the toolkit changed.* The checked-in captures were drawn by
GTK 4.18.6 and libadwaita 1.7.6, which is what their rows in `docs/design/captures/INDEX.md`
record, because the old build container was Debian trixie and trixie's toolkit sits above the floor.
The private runtime pins GTK 4.16.7 and libadwaita 1.6.9. A different toolkit draws differently, so
every capture is stale whatever its text says. That is measured rather than assumed:
`skew_attention-light.png` redrawn against the shipping runtime is 60,960 bytes against 62,439
checked in, with its copy untouched.

The rendering above is settled, because the toolkit bytes in that image are the frozen ones. The
runtime key those captures record in `INDEX.md` is not: slice 1's freeze is still open pending the
GL-renderer smoke the team lead has required before release. So the re-take lands in a scratch
directory until that key is final, and this document deliberately names no key. A capture row
carrying a key that later moves is worse than a capture row written a day later.

This is the cost of section 4.4's decision to pin the bundle at exactly the compile floor, and it is
worth paying once here. From this amendment on the captures are drawn by the toolkit that ships to
users, not by whatever the build image happened to carry, which makes a reviewed capture a
statement about the product for the first time.

*The three copy edits need no capture review. This is settled by reading the drawn image, not by
tracing the call graph.* Three attempts were made to establish it from the source and two of them
produced wrong answers, so the method matters as much as the answer and is recorded with it.

`skew_attention-light.png`, redrawn against the shipping toolkit, renders the Home surface: an
Attention list of three rows, a Background card and a Finish Updating action. The three rows are the
unfinished WhatsApp channel, the older service file at
`~/Library/LaunchAgents/io.tezra.fermix.plist`, and `Key::SkewNewerInstalledTitle` with its body.
None of the three edited sentences is on it. That is the whole finding.

The supporting facts, none of them load-bearing on their own:

| Key | Where it renders |
|---|---|
| `RecoveryPackageRepair` | Nowhere. No render site outside `src/copy.rs`, and this one is machine-checked rather than grepped: the key is listed in `tests/fixtures/copy/keys_awaiting_a_surface.txt`, which the copy gate maintains so a key with no surface is a recorded state rather than an absence somebody inferred |
| `PreflightPreManagementDaemonBody` | `src/models/activation.rs:545`, on the preflight refusal path, reached only when `status.answers_nothing()` |
| `ActivationProtocolMismatchBody` | `src/models/activation.rs:679`, via `protocol_refusal`. The sole skew fixture could not reach it in any case: its `hello` advertises protocol `current_version` 2, `minimum_version` 1, `maximum_version` 2, which overlaps the window's range, so `protocol_refusal` returns `None` |

The two preflight keys are absent from the capture for a simpler reason than any branch order: no
capture reference arranges the preflight refusal path at all. `skew_attention` arranges Home. An
earlier draft of this section said it takes the foreign-distribution branch of
`preflight_of_status`; that was a prediction from the fixture's
`distribution_identity` of `macos_app`, and the rendered image falsified it. The fixture is still
not a protocol skew, which is worth knowing when reading its name, but what it draws is a Home
surface and not a refusal.

Three readings of this question gave two wrong answers before the image gave the right one: a grep
whose pattern did not match the key it was meant to find, and a call-graph trace of a surface the
capture does not route to. The conclusion survived all three unchanged, which is luck rather than
method. Where a question is about what a person sees, the answer comes from the picture.

*The scenario list is unchanged*, and it lives in `App/Fermix/src/capture.rs`, not in
`scripts/capture.sh`. No scenario depends on two packages.

**`src/copy.rs` casing classes and the copy gate.** All three edits keep their `Casing::Sentence`
class and none introduces an em dash, an exclamation mark, a version number or a wire token, so
`scripts/check_copy.sh` passes unchanged.

## 8. Build and test

### 8.1 Containers

| File | Fate |
|---|---|
| `packaging/docker/Dockerfile.runtime` | **New.** `FROM almalinux:9` pinned by digest. Builds the private runtime. Its output is published as an OCI image and is the cache of section 4.2 |
| `packaging/docker/Dockerfile.build` | **Rewritten.** `FROM almalinux:9` pinned by digest, with the private runtime copied in from the runtime image, the pinned Rust toolchain, nFPM pinned by digest as today, and the host-side development headers. `libgtk-4-dev` and `libadwaita-1-dev` are gone: the crate builds against the private prefix through `PKG_CONFIG_PATH` |
| `packaging/docker/Dockerfile.smoke` | **Rewritten as a matrix.** Parameterised by base image, so one Dockerfile produces the systemd smoke container for each row of section 8.4 |

### 8.2 Scripts

| Script | Fate |
|---|---|
| `packaging/runtime/build_runtime.sh` | **New.** Section 4.2 |
| `packaging/runtime/RUNTIME.lock.json`, `packaging/runtime/patches/` | **New** |
| `scripts/check_no_network.sh`, `scripts/check_copyright.sh` | **New.** Section 4.3, section 4.6 |
| `scripts/check_private_runtime.sh` | **New.** Over the staged tree: every private object's `RUNPATH` is exactly `$ORIGIN/../lib`; every `NEEDED` entry resolves either inside the prefix or to a library on section 4.1's host list; no private object names a host GTK, GLib or pango; the pixbuf cache and the compiled schemas name only private paths |
| `scripts/build_packages.sh` | **Rewritten.** Accepts `X.Y.Z+N`; requires a pinned engine; unpacks the verified engine tree into the stage; concatenates `nfpm-contents.yaml` into the rendered configuration; copies the private runtime out of the runtime image; runs the new gates. The version, build-id and record-before-machine discipline is kept verbatim |
| `scripts/package_dependencies.py` | **Rewritten.** The question is inverted. Today it proves the declared toolkit relations cover the binary's needs. Now it proves the opposite: that **no** `NEEDED` entry anywhere in the package resolves outside the private prefix except to a declared host relation, and that the glibc floor the ELFs require is not above the declared `libc6 (>= 2.34)`. Its fixture-driven offline test shape is kept |
| `scripts/fetch_engine.sh`, `scripts/verify_engine.sh`, `scripts/engine_pin.sh` | **Rewritten** for schema 2, section 5.4 |
| `scripts/install_smoke.sh` | **Rewritten.** One package argument, not two. A `--image` flag selects the row of the matrix |
| `scripts/container_build.sh`, `scripts/capture.sh` | **Amended.** They use the rewritten build image and otherwise keep their shape |
| `scripts/verify_contract.sh`, `check_app_identity.sh`, `check_copy.sh`, `check_vendor_marks.sh`, `render_icons.sh`, `render_install_notes.sh`, `vendor_marks.py` | **Unchanged** |
| Every `*_test.sh` beside the above | Follows its script |

Nothing is deleted outright.

### 8.3 Gates

`app.yml` on every push: the existing gates, plus `check_no_network.sh`. `packages.yml`: the existing
package build, plus `check_private_runtime.sh`, `check_copyright.sh`, the rewritten
`package_dependencies.py`, and `build_runtime.sh --verify` on a schedule rather than per push.

### 8.4 The install smoke matrix

| Image | Family | Arch | What it proves |
|---|---|---|---|
| `ubuntu:22.04` | deb | amd64, arm64 | The oldest deb target. GTK 4.6 on the host and the window still draws |
| `ubuntu:24.04` | deb | amd64, arm64 | |
| `ubuntu:26.04` | deb | amd64 | The newest deb target, and the deb family's headless Wayland row |
| `debian:12` | deb | amd64 | |
| `debian:13` | deb | amd64 | |
| `almalinux:9` | rpm | amd64 | The oldest rpm target, and the glibc floor |
| `fedora:44` | rpm | amd64 | The newest rpm target, and the rpm family's headless Wayland row |

Each row: systemd as process one, install the one package through the family's package manager,
grant linger to an ordinary account, run `fermix service install --json --home "/home/test/fermix home"`
and `fermix service status --json` as that account, then open the window on a display and photograph
it. Evidence lands in `packaging/out/smoke/<image>/`. The arrangement that linger is granted by root
before the install runs is unchanged and still stated, because the polkit hop is the engine suite's
to prove.

**Two displays, not one.** Every row draws the window under Xvfb, which exercises the X11 backend.
Two rows, `ubuntu:26.04` and `fedora:44`, draw it a second time under `weston --backend=headless-backend.so`
with `WAYLAND_DISPLAY` set and `GDK_BACKEND=wayland`, and photograph that too. Xvfb alone proves
nothing about the private `libwayland-client`, which is the component this design replaces at a
version the host does not have and the component whose failure mode is a window that never appears.
Wayland is the default session on most of the matrix, so a smoke that only ever runs X11 tests the
minority path. The two rows are the newest of each family because those are the images that carry
`weston` and `weston-screenshooter` without a backport, which slice 6 confirmed by building the
smoke container on all seven; the older images have neither. The oldest images keep the X11 run
only, and the gap is named rather than closed, because Ubuntu 22.04's Wayland path is covered by the
acceptance runbook's real-session rows.

**The image names live in `docs/RELEASING.md`, and that document is the authority for this matrix.**
Distribution releases go end of life faster than a design document is re-read, and this table was
wrong within months of being written. `scripts/install_smoke_test.sh` reads the image names out of
`docs/RELEASING.md` and refuses one the gate has no package manager for, so the gate and that
document cannot drift. This section states the shape of the matrix, which is stable: oldest and
newest of each family, both arches on the two Ubuntu LTS rows, and headless Wayland on the newest of
each family. The names in the table above are correct as of 2026-09-19 and are a copy.

### 8.5 A developer on Pop!\_OS 22.04

```sh
git clone git@github.com:tezra-io/fermix-linux.git
cd fermix-linux
scripts/container_build.sh                      # gates, inside the build image
scripts/build_packages.sh 0.11.0 amd64 --container
sudo apt install ./packaging/out/fermix-desktop_0.11.0_amd64.deb
```

The first `container_build.sh` pulls the runtime image rather than building it, because the published
image for the current lock file exists. A developer who changes `RUNTIME.lock.json` runs
`packaging/runtime/build_runtime.sh --container` once, which takes about forty minutes on a laptop
and is then cached in a named Docker volume the same way the cargo cache is.

`build_packages.sh` requires a pinned engine, so a developer on an unpinned tree passes
`--engine <path to an archive>` to build against a local one. That flag exists for exactly this and
is refused in CI.

**That archive comes from the engine checkout, and building it must not need Elixir on the
developer's machine.** This machine has no Elixir, no Erlang and no Zig, and installing them to see
a window change would be the wrong trade. `scripts/release/build_app_engine.sh` in `tezra-io/fermix`
therefore gains the two flags `scripts/release/build_linux_packages.sh` already has:

```sh
cd ../fermix
scripts/release/build_app_engine.sh linux_amd64 /tmp/engine 0.10.5 --container --dev
cd ../fermix-linux
scripts/build_packages.sh 0.10.5 amd64 --container --engine /tmp/engine/fermix_app_engine_linux_x86_64.tar.gz
```

`--container` runs the whole build inside `packaging/linux/docker/Dockerfile.build`, which already
carries the toolchain, so the host needs only Docker. `--dev` relaxes exactly the three release
refusals that cannot hold locally and nothing else: it allows a dirty checkout, it allows
`FERMIX_BUILD_SOURCE_COMMIT` to differ from `HEAD`, and it stamps a build id of `dev-<short sha>`.
It does not relax the manifest validation, the loader-digest cross-check or the tree digest, so a
`--dev` archive is still a structurally valid archive and `verify_engine.sh` still refuses it
against a real pin, which is what keeps it out of a release. Section 5.3 and slice 2 own the flags.

The installed package runs on Pop!\_OS 22.04 with its GTK 4.6, which is the whole point, and
`ldd /usr/lib/fermix-desktop/bin/fermix-desktop` shows the private prefix.

## 9. Updates

The apt and dnf repositories of M38 §2.3 remain the update channel and are not built by this
amendment. Until they exist, a person updates by downloading the next package and installing it over
the current one, which dpkg and rpm both handle as an upgrade.

What the application does in the meantime is what M38 §9.4 already specifies and what is already
built: the check-only notifier reads the engine's release feed, says that a newer version exists,
and offers no in-app update. The one change is its sentence, which named two packages and now names
one. There is no Sparkle, no in-app downloader and no self-update, and each absence is deliberate:
the product installs as a system package and a system package updates through the system.

## 10. Open questions

Each of these carries a provisional default, recorded by the coordinating session so that
implementation is not blocked waiting on them. A default is what happens if nobody says otherwise;
it is not the owner's answer, and any of the four can be reversed without reopening the design.

1. **The published size.** The private runtime is roughly 90 MB installed before the engine, which
   is itself roughly 120 MB. A combined package near 210 MB installed and 70 MB compressed is what a
   person sees next to the Download button.
   *Provisional default: accepted.* It is the price of running on Ubuntu 22.04 and it is in the
   range every comparable product occupies.
2. **openSUSE Leap.** Leap 15.6's glibc 2.31 is below the floor, and covering it would mean a second
   build base and a second runtime image for one distribution.
   *Provisional default: unsupported*, served by the standalone binary, as section 2 already states.
3. **Adwaita icon theme, host-first or bundle-first.** Host-first keeps the window looking like the
   session and lets a host with an old or customised Adwaita change how it looks. Bundle-first makes
   every machine identical and looks foreign on a themed desktop. It is a product judgement, not a
   technical one.
   *Provisional default: host-first*, which is what section 4.3's `add_search_path` implements.
4. **Whether the bot's pull request needs a human reviewer.** The argument for none is that the full
   check set is the review and the `release-linux` environment is the gate that matters. A reviewer
   is cheap to add and costs a person per engine release.
   *Provisional default: no human reviewer on the bot pull request*, with the `release-linux`
   environment kept as the single human gate.

A fifth question is decided rather than open, and is recorded here because it will be asked:
**Arch and Omarchy ship no package in v1**, section 2.1, with slice 7 recommended as the first
post-v1 slice.

## 11. Implementation slices

Six slices to ship, and a seventh named for after. No two own the same file.

**Slice 1: the private runtime.**
Owns `packaging/runtime/` (all of it), `packaging/docker/Dockerfile.runtime`,
`.github/workflows/runtime.yml` (build and publish the runtime image).
Acceptance: `packaging/runtime/build_runtime.sh --container && packaging/runtime/build_runtime.sh --verify`
passes, and `pkg-config --modversion gtk4 libadwaita-1` inside the produced prefix prints a 4.16.x
and a 1.6.x.
Depends on nothing.

**Slice 2: the engine artifact, in the fermix repository.**
Owns `fermix/scripts/release/build_app_engine.sh`, `package_app_engine.py`, `verify_app_engine.py`,
`verify_app_engine.sh`, `linux_packages.py`, their three test files, and the `app-engine` matrix in
`fermix/.github/workflows/release.yml`.
Also owns the `--container` and `--dev` flags of section 8.5.
Acceptance: `python3 scripts/release/test_package_app_engine.py` passes, and
`scripts/release/build_app_engine.sh linux_x86_64 /tmp/out 0.10.5 --container --dev && scripts/release/verify_app_engine.sh /tmp/out/fermix_app_engine_linux_x86_64.tar.gz`
passes on a host that has Docker and no Elixir, which is the machine this work is being done on.
Depends on nothing. Can run in parallel with slice 1.

**Slice 3: the pin and the engine-side scripts here.**
Owns `engine/PIN.json`, `scripts/engine_pin.sh`, `scripts/fetch_engine.sh`, `scripts/verify_engine.sh`
and `scripts/verify_engine_test.sh`.
Acceptance: `scripts/verify_engine_test.sh` passes, including a case that refuses a manifest whose
`source_commit` is not the pin's.
Depends on slice 2 for the artifact shape; can be written against a fixture before slice 2 lands.

**Slice 4: the package.**
Owns `packaging/nfpm-fermix-desktop.yaml.tmpl`, `packaging/scripts/postinstall.sh` and
`postremove.sh`, `packaging/docker/Dockerfile.build`, `packaging/copyright`, `packaging/INSTALL.md.tmpl`,
`scripts/build_packages.sh`, `scripts/build_packages_test.sh`, `scripts/package_dependencies.py`,
`scripts/check_private_runtime.sh`, `scripts/check_copyright.sh`, `scripts/container_build.sh`.
Acceptance: `scripts/build_packages.sh 0.11.0 amd64 --container --engine <fixture>` produces a deb
and an rpm, and `scripts/check_private_runtime.sh packaging/out/stage` passes.
Depends on slices 1 and 3.

**Slice 5: the application.**
Owns `App/Fermix/src/runtime.rs`, `App/Fermix/tests/runtime_env.rs`,
`src/main.rs`, `src/service/runner.rs`, `src/session/desktop.rs`,
`src/copy.rs`, `src/paths.rs`, `src/capture.rs`, `App/Fermix/tests/packaging.rs`,
`scripts/check_no_network.sh`, `scripts/capture.sh`, and `docs/design/captures/`.
Acceptance: `scripts/container_build.sh` passes, including a new unit test asserting a spawned
child's environment equals the process's entry environment.
Depends on slice 4 for the build image, and on nothing else. The copy and test edits can land first.

**Slice 6: automation and documentation.**
Owns `.github/workflows/engine-release.yml` (new), `release-fermix-desktop.yml`, `packages.yml`,
`app.yml`, `runtime-watch.yml` (new), `scripts/install_smoke.sh`, `scripts/install_smoke_test.sh`,
`docs/RELEASING.md`, `docs/ACCEPTANCE_RUNBOOK.md`, `CLAUDE.md`, `README.md`, and the
`dispatch-linux-desktop` job in `fermix/.github/workflows/release.yml`.
Acceptance, three things. A dry-run `repository_dispatch` against a staging tag opens a pull request
that actually starts its checks, which is what proves the App token of section 6.1 and would have
been silently green with `GITHUB_TOKEN`. `scripts/install_smoke.sh --image ubuntu:22.04 packaging/out/fermix-desktop_0.11.0_amd64.deb`
passes, and the `ubuntu:26.04` and `fedora:44` rows pass under headless Weston as well as Xvfb. And
a full `0.11.0+1` dry run goes build, sign, upload to a draft release, download by name and install,
per section 6.4, resolving the asset-name question one way or the other.
Depends on slices 4 and 5.

**Slice 7: Arch and Omarchy. Post-v1, recommended first.**
Owns `packaging/nfpm-fermix-desktop-arch.yaml.tmpl` (the `archlinux` packager's relation dialect is
the only thing that differs from the shared template), `packaging/aur/PKGBUILD.tmpl`, the
`archlinux` row of `scripts/package_dependencies.py`, the `archlinux:latest` row of the smoke
matrix, and the AUR publish job in `release-fermix-desktop.yml`.
Acceptance: `scripts/install_smoke.sh --image archlinux:latest packaging/out/fermix-desktop-0.11.0-1-x86_64.pkg.tar.zst`
passes, with the added assertion that the window reports the host's `color-scheme` through the
bundled dconf backend on a session with no Settings portal.
Depends on slices 4, 5 and 6. Owns no file any other slice owns.

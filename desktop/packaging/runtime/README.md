# The private toolkit runtime

`fermix-desktop` carries its own GTK 4.22, libadwaita 1.9, GLib 2.88 and
GStreamer 1.26, and everything under them the host cannot be relied on to
supply, in a private prefix at `/usr/lib/fermix-desktop`. This directory builds
that prefix. The plan is `fermix-linux/single_package.md` in the design repo;
the rules it keeps are sections 4.1 to 4.3 of
`fermix-linux/earlier-client/SINGLE_PACKAGE_AMENDMENT.md` (A§n below).

```
RUNTIME.lock.json         every component: version, url, sha256, build flags, licence,
                          the Cargo.lock of a Rust one;
                          the host library list and the symbol version ceilings
build_runtime.sh          fetch, build, wire, check and export the prefix
check_boundary.sh         the host boundary gate, on any prefix
fetch_source.sh           one fetch and one digest rule, shared by the build and the archive
package_sources.sh        the LGPL source archive published beside the packages
smoke_runtime.sh          draw a libadwaita window on ubuntu:22.04 from the built tree
smoke/runtime_smoke.c     the window it draws
write_manifest.py         runtime-manifest.json: the lock plus every shipped file's sha256
compare_manifest.py       what --verify compares
check_options.py          holds every meson flag to the option upstream declares
build_runtime_test.sh     the offline tests; with build_runtime_test_lock.py and
                          build_runtime_test_boundary.sh
patches/                  per-component patches, named by the lock file
../docker/Dockerfile.runtime   the toolchain image the build runs in
```

## Running it

```sh
desktop/packaging/runtime/build_runtime_test.sh             # offline tests, seconds
desktop/packaging/runtime/build_runtime.sh --container      # build, export the trees
desktop/packaging/runtime/build_runtime.sh --container --fresh   # from empty volumes
desktop/packaging/runtime/smoke_runtime.sh                  # the window on ubuntu:22.04
desktop/packaging/runtime/build_runtime.sh --verify         # rebuild and compare
desktop/packaging/runtime/build_runtime.sh --print-key      # the cache key, no daemon
desktop/packaging/runtime/package_sources.sh 0.12.2         # the source archive
```

On six cores the build takes about 10 minutes from empty volumes, and about 25
when the image is built too. It resumes: each installed component leaves a
stamp in the build volume, and the volumes carry the cache key they were filled
under, so a changed lock file, patch, Dockerfile, `build_runtime.sh` or
`write_manifest.py` starts from empty and an unchanged set picks up where a
failure stopped.

Outputs land in `$FERMIX_RUNTIME_OUT`, by default
`~/.cache/fermix-desktop-runtime/out`:

| File | What |
|---|---|
| `runtime-<arch>.tar` | The shipped tree, rooted at `usr/lib/fermix-desktop` |
| `runtime-dev-<arch>.tar` | The same tree with headers, `.pc` files and the build-time binaries |
| `runtime-manifest.json` | The lock file plus the sha256 of every shipped file and of both tarballs |
| `SHA256SUMS` | Written in the container, checked on the host after the copy |
| `cache-key` | The key of A§4.2 |
| `smoke/smoke-{cairo,gl}.png` | The smoke's screenshots |

Other variables: `FERMIX_RUNTIME_SOURCES` moves the tarball cache (default
`~/.cache/fermix-desktop-runtime/sources`), `FERMIX_RUNTIME_JOBS` sets the
compile parallelism (default 4), `FERMIX_RUNTIME_IMAGE`,
`FERMIX_RUNTIME_PREFIX_VOLUME` and `FERMIX_RUNTIME_BUILD_VOLUME` name the image
and the two volumes, all `fermix-desktop-pkg-runtime-*` by default.
`DOCKER_HOST` picks the daemon; the scripts never name one.

## What the package and the window need from this

**The package.** Unpack `runtime-<arch>.tar` at the root of the staged tree:
it is already `usr/lib/fermix-desktop/...`, pruned of headers, `.pc` and CMake
files, static libraries, documentation, the MIME database and every build-time
binary. There is no `bin/`; the application ELF's directory is the package's to
create. `libexec/` stays for `gio-launch-desktop` and `gst-plugin-scanner`,
which GLib and GStreamer run by their compiled-in paths. Nothing needs a
post-install step: `loaders.cache`, `giomodule.cache`, `gschemas.compiled` and
the icon cache are generated at build time against the final prefix.
`runtime-manifest.json` ships at
`/usr/share/doc/fermix-desktop/runtime-manifest.json`; its `lock` object carries
each component's `license`, which the copyright file is generated from.
`check_boundary.sh <dir> RUNTIME.lock.json` runs the same gate on any tree.

**The window.** Build against the dev tree, unpacked at `/`:

```sh
export PKG_CONFIG_PATH=/usr/lib/fermix-desktop/lib/pkgconfig:/usr/lib/fermix-desktop/share/pkgconfig
export PATH=/usr/lib/fermix-desktop/bin:$PATH   # glib-compile-resources, for build.rs
```

Link the ELF with `-Wl,-rpath,'$ORIGIN/../lib' -Wl,--enable-new-dtags`; it
lives at `/usr/lib/fermix-desktop/bin/fermix-desktop`. `smoke_runtime.sh`
compiles its C program with exactly that link line.

At run time the window sets `GSETTINGS_SCHEMA_DIR` to
`/usr/lib/fermix-desktop/share/glib-2.0/schemas` (it prepends, so the host's
`org.gnome.desktop.interface` is still found) and appends
`/usr/lib/fermix-desktop/share/icons` with `gtk::IconTheme::add_search_path`,
because GTK builds its icon search path from `XDG_DATA_DIRS`, not from its
prefix. It sets no `GIO_MODULE_DIR`, `GDK_PIXBUF_MODULE_FILE`,
`GST_PLUGIN_SYSTEM_PATH` or `XDG_DATA_DIRS`: GLib, gdk-pixbuf and GStreamer have
the prefix compiled in.

## Decisions, and the evidence for them

**Versions.** The window builds `gtk4` 0.11 with `gnome_50` and `libadwaita`
0.9 with `v1_9`. In the `-sys` crates' metadata `gnome_50` means `v4_22`, which
asks pkg-config for gtk4 `>= 4.21`, and `gio/v2_88`, glib `>= 2.88`; `v1_9` asks
for libadwaita-1 `>= 1.9`. Pinned: GTK 4.22.5, libadwaita 1.9.4 and GLib 2.88.3,
the newest point release of each series. Every other version meets what the
pinned versions' own `meson.build` requires:

| Required by | Requirement read from upstream | Pinned |
|---|---|---|
| GTK 4.22.5 | glib >= 2.84, pango >= 1.56, harfbuzz >= 8.4.0, fribidi >= 1.0.6, cairo >= 1.18.2, gdk-pixbuf >= 2.30.0, wayland >= 1.24.0, wayland-protocols >= 1.44, graphene >= 1.10.0, epoxy >= 1.4, xkbcommon >= 0.2.0 (host), meson >= 1.5.0; libpng, libjpeg and libtiff are hard dependencies of its own image loaders | pango 1.56.4, harfbuzz 14.4.0, fribidi 1.0.16, cairo 1.18.4, gdk-pixbuf 2.42.12, wayland 1.26.0, wayland-protocols 1.49, graphene 1.10.8, libepoxy 1.5.10, libpng 1.6.58, libjpeg-turbo 3.2.0, libtiff 4.7.2 |
| libadwaita 1.9.4 | glib >= 2.84, gtk4 >= 4.21.1, fribidi, appstream (unconditional) | appstream 1.0.6 |
| pango 1.56.4 | glib >= 2.82, fontconfig >= 2.15.0, cairo >= 1.18.0 | fontconfig 2.16.0 |
| cairo 1.18.4 | freetype >= 2.13 for COLRv1, fontconfig >= 2.13.0, pixman >= 0.40.0, libpng >= 1.4.0 | freetype 2.13.3, pixman 0.46.4 |
| fontconfig 2.16.0 | freetype >= 21.0.15 (2.10.2), an XML parser | expat 2.8.4 |
| appstream 1.0.6 | glib >= 2.62, libxmlb >= 0.3.14, libxml2, libyaml and libcurl >= 7.62 (host) | libxmlb 0.3.29, libxml2 2.13.9 |
| GLib 2.88.3 | pcre2 >= 10.32, libffi >= 3.0.0, meson >= 1.4.0, Python >= 3.7 | pcre2 10.48, libffi 3.8.0 |
| webp-pixbuf-loader 0.2.7 | gdk-pixbuf > 2.22.0, libwebp >= 1.3.2 | libwebp 1.6.0 |
| librsvg 2.62.4 | cairo >= 1.18.0, pango >= 1.50.0, glib >= 2.50.0, harfbuzz >= 2.0.0, freetype2 >= 20.0.14, libxml2 >= 2.9.0, gdk-pixbuf >= 2.20, meson >= 1.3.0, Rust >= 1.92.0, cargo-c >= 0.10.10 | Rust 1.97.1 and cargo-c 0.10.24 in the build image |
| GStreamer 1.26.11 | glib >= 2.64, meson >= 1.4, flex >= 2.5.31 | |
| gst-plugins-bad 1.26.11 webrtcdsp | webrtc-audio-processing-2 >= 2.0 | webrtc-audio-processing 2.1 |
| webrtc-audio-processing 2.1 | absl_base >= 20240722, C++17; its own wrap pins abseil-cpp 20240722.0 | abseil-cpp 20240722.2 |

Pins carried over from the earlier client's lock file stay wherever they still
meet these, because that set has built together; each one that moved, moved
because a requirement above moved it. adwaita-icon-theme is 50.0, the set GTK
4.22 and libadwaita 1.9 ship with and the one the window's neutral icons are
named from. librsvg is 2.62.4, the newest point release of the series GNOME 50
ships; 2.63 is GNOME 51's. GStreamer is 1.26.11, the newest 1.26 point release.

**No glycin; librsvg for the SVG GTK cannot draw.** GTK 4.22.5's
`meson.options` has no glycin option and its `meson.build` never looks for
glycin: `gdk-pixbuf-2.0` is a hard dependency, and libpng, libjpeg and libtiff
are too. `gdk/gdktexture.c` decodes PNG, JPEG and TIFF with GTK's own loaders
and hands every other format to gdk-pixbuf (`gdk_texture_new_from_bytes_pixbuf`);
glycin appears only in documentation comments recommending it to applications.
gdk-pixbuf is pinned at 2.42.12, which predates gdk-pixbuf's own glycin
support, and built with the PNG and JPEG loaders only (`-Dtiff=disabled
-Dgif=disabled -Dothers=disabled`), plus webp-pixbuf-loader for WebP and
librsvg's loader for SVG. So nothing is sandboxed, no bubblewrap and no loader
binaries are needed, and the host needs nothing for images.

SVG is GTK's own first. In 4.22 the icon theme draws SVG icons with `GtkSvg`
and falls back to the image loader only when `GtkSvg` reports a feature it does
not implement (`gtk/gdktextureutils.c`), and `desktop/app/src/marks.rs` follows
the same rule. That fallback is librsvg's: built without it, the smoke, which
decodes every file under `desktop/app/marks/` the way `marks.rs` does, drew 34
marks and refused `channels/discord-white.svg` and
`channels/discord-blurple.svg`, whose `<style>` element `GtkSvg` does not
implement (`desktop/app/marks/PROVENANCE.json` records the same). A§4.1 lists
librsvg as private for this reason. It is built as the pixbuf loader and its
library only (`-Drsvg-convert=disabled -Davif=disabled`), and it costs a Rust
toolchain and cargo-c in the build image.

**Crates.** librsvg's Rust half needs 357 crates that its tarball does not
carry. They are pinned by the `Cargo.lock` inside that tarball, which the lock
file pins by sha256 and names in librsvg's `cargo_lock` field. For a component
with that field, `vendor_crates` fetches the crates with `cargo vendor
--locked`, which refuses one whose checksum differs from `Cargo.lock`, into the
source tree; the compile then runs with `CARGO_NET_OFFLINE=true`. It is the only
network fetch the build makes itself. A `Cargo.lock` alone does not trigger it:
fontconfig 2.16's tarball carries one for its optional Fontations backend, which
is not built. The build log lists the vendored crates by licence. Six are
MPL-2.0, which asks that recipients of the binary be told where the source is;
the source archive points to them through librsvg's `Cargo.lock` and does not
carry them.

**Voice.** The window's pipelines (`desktop/app/src/audio.rs`) make
`pulsesrc`, `pulsesink`, `webrtcdsp`, `webrtcechoprobe`, `audioconvert`,
`audioresample`, `capsfilter`, `audiotestsrc`, `fakesink`, `appsrc` and
`appsink`. Each GStreamer module is configured with `-Dauto_features=disabled`
and enables only the plugins that provide those: `tools` in core (for
`gst-inspect-1.0`), `app`, `audioconvert`, `audioresample` and `audiotestsrc`
in base, `pulse` and `level` in good, `webrtcdsp` in bad. The lock file test
refuses any other enable. `level` is built because the plan names it;
`audio.rs` computes its own meter today. The libraries each module always builds
(gst-plugins-bad's `gst-libs`, for one) are built and shipped; nothing loads
them unless something links them.

**abseil is static.** webrtc-audio-processing 2.1 needs abseil and would
otherwise download it as a meson subproject mid-build. abseil is a locked
component instead, built by CMake as position-independent static archives and
linked into `libwebrtc-audio-processing-2.so.1`, so the shipped tree has no
`libabsl_*` files. Its code still ships inside that library, so it keeps its
licence entry.

**The host boundary.** `host_libraries` is A§4.1's host column, plus
`libpulse.so.0`, the PulseAudio client protocol and so the session's, and
`libstdc++.so.6`, which webrtc-audio-processing and the pet's renderer link.
`libpulsecommon-<version>.so` is never on it: its SONAME carries the PulseAudio
version, so no one name exists on every host, and only `libpulse.so.0` links it.
`check_boundary.sh` refuses any `NEEDED` entry that is neither a file in the
prefix's `lib/` nor on that list, and any object needing a symbol version above
the oldest host's: `GLIBC_2.34` (the build base, AlmaLinux 9), and
`GLIBCXX_3.4.30` and `CXXABI_1.3.13`, the newest that Ubuntu 22.04's libstdc++6
12.3.0 defines. It runs after every component, so the component that crossed
the boundary is the one named, and again over the finished tree. It cannot see
a library loaded with `dlopen`; the smoke's GL pass reaches libepoxy's.

The list is a floor, not a census. The GL family (`libGL`, `libEGL`,
`libGLESv2`, `libGLESv1_CM`, `libGLX.so.0`, `libOpenGL`) is loaded by libepoxy
with `dlopen` and must never be pruned because no `NEEDED` entry names it.
libepoxy asks for `libGLX.so.1`, which no distribution ships; the list records
`libGLX.so.0`, glvnd's real name, and epoxy falls back to `libGL.so.1`.

**The build image carries no private headers.** `pulseaudio-libs-devel`
requires `glib2-devel`, which brings libffi, pcre2, sysprof, libmount and
libselinux headers, and `libxkbcommon-devel` requires `libxml2-devel`. Those are
removed again with `rpm -e --nodeps`, and the next image step fails if a `.pc`
file of any private component is left on the host, so a component can only ever
find the private copy. The image is AlmaLinux 9.8 by digest, with GCC 11, Meson
1.12.1, Ninja 1.13.2, patchelf 0.18.0, sassc 3.6.2, Rust 1.97.1 from rustup-init
1.29.1 (pinned by digest), cargo-c 0.10.24, whose embedded cargo is the one
Rust 1.97 ships, and `shared-mime-info`, whose `update-mime-database` GTK's
install step runs.

**Meson never downloads.** Every meson component is configured with
`--wrap-mode=nodownload`, so a dependency the prefix lacks fails the build
rather than fetching an unpinned subproject.

**No bind mounts.** The build and the smoke copy their inputs into a created
container with `docker cp` and copy their outputs out the same way; the build
container writes `SHA256SUMS` and the host checks the copies against it. A
bind-mounted directory on a daemon in a virtual machine carries another clock,
which stops ninja on "Clock skew detected", and can truncate large files written
through it. The prefix and the build tree are named volumes, written by the
daemon itself.

**RUNPATH is written once, by patchelf.** `fix_runpaths` gives every object
the relative path to `lib/` from where it sits: `$ORIGIN` in `lib/`,
`$ORIGIN/../lib` in `bin/`, `$ORIGIN/../../../../lib` for a pixbuf loader.
`check_runpaths` reads every one back and refuses an `RPATH`.

**Smaller choices.** HarfBuzz builds no `raster`, `vector` or `gpu` library,
which nothing here links, and no libstdc++ (`-Dwith_libstdcxx=false`). libtiff
is `--disable-cxx`. GTK has no Vulkan renderer (it would link `libvulkan.so.1`,
which is not on the host list), no GStreamer media backend, no print backends
and no AccessKit. FreeType is `-Dharfbuzz=disabled`, which breaks a dependency
cycle at the cost of script-aware autohinting. The private fontconfig reads the
host's `/etc/fonts` and shares `~/.cache/fontconfig`, whose file names carry
the cache format version. From dconf only the GSettings module and its library
ship; the host runs `dconf-service`.

**Reproducibility.** `SOURCE_DATE_EPOCH` is the lock file's
`source_date_epoch`, the compiler is pinned by the base image digest, the prefix
is a fixed path, `-ffile-prefix-map` and `--remap-path-prefix` keep the build
directory out, and `check_no_build_paths` refuses a tree that names it. Annobin
notes and Python bytecode are removed because they vary between two builds of
one tree. librsvg is patched so the order it links its system libraries in no
longer follows Python's hash seed
(`patches/librsvg-native-libs-in-rustc-order.patch`), and CMake writes
abseil's static archives in `ar`'s deterministic mode. `tar --sort=name` makes
each archive's digest a property of the tree. `--verify` rebuilds from empty
volumes and compares file by file and archive by archive.

## Deviations from the amendment

| Amendment | What is true | Why |
|---|---|---|
| A§3.2 ships `lib/gtk-4.0/4.0.0/immodules/*.so` | GTK 4 has no loadable input modules | `GtkIMContextSimple` and the Wayland `text-input-v3` context are compiled into libgtk |
| A§4.1 lists neither expat nor pixman | Both are private | fontconfig needs an XML parser and cairo needs pixman |
| A§4.1 keeps libstdc++ off the host list | It is on it | webrtc-audio-processing is C++, and the plan adds it with a symbol ceiling |
| A§4.2 says `build_system` is `meson`, `autotools` or `cargo` | `meson`, `autotools` or `cmake` | libjpeg-turbo and abseil build with CMake; librsvg's meson build drives cargo itself |
| A§4.2 computes the cache key from the lock file, the patches and the Dockerfile | It also covers `build_runtime.sh` and `write_manifest.py` | The script sets the compiler, linker and archiver flags and the manifest is published with the tree; a change to either under an unchanged key would publish a different runtime under the old key |
| A§4.2 builds the runtime in an image layer | The image is the toolchain; the compile is a container run | A failed component resumes, and the source cache is copied in rather than re-fetched |
| A§4.2 sets `SOURCE_DATE_EPOCH` from the lock file's commit | It is the lock file's `source_date_epoch` field | A commit date moves under a rebase; a field is covered by the cache key |
| A§4.3 puts the fontconfig cache under `$XDG_CACHE_HOME/fermix-desktop/fontconfig` | It shares `~/.cache/fontconfig` | The host `fonts.conf` names the cache directory and wins over a compiled-in default |

## Bumping a component

1. Change the version, url and sha256 in `RUNTIME.lock.json`. Fetch the tarball
   and read its digest; never copy one from a web page.
2. Read the requirements in the new version's `meson.build` and in the
   `meson.build` of everything that depends on it.
3. `build_runtime_test.sh`. With the tarball in the source cache it also holds
   every meson flag to the component's own option file.
4. `build_runtime.sh --container`, then `smoke_runtime.sh`.

A GTK or libadwaita point release can change rendering, so the window's
reviewed captures are taken again after one.

# The private toolkit runtime

`fermix-desktop` carries its own GTK 4.22, libadwaita 1.9, GLib 2.88 and
GStreamer 1.26, and everything under them the host cannot be relied on to
supply, in a private prefix at `/usr/lib/fermix-desktop`. This directory builds
that prefix. The plan is `fermix-linux/single_package.md` in the design repo;
the rules it keeps are sections 4.1 to 4.3 of
`fermix-linux/earlier-client/SINGLE_PACKAGE_AMENDMENT.md` (A§n below).

```
RUNTIME.lock.json         every component: version, url, sha256, build flags, licence,
                          licence files outside the top of its tree, notices inside
                          its source files, the file of each LicenseRef's text, the
                          standard texts its licence needs, and for a Rust one its
                          Cargo.lock and packages; the host library list, the symbol
                          version ceilings, the pkg-config packages the window
                          links, and from SPDX's license-list-data, pinned by
                          sha256, the standard texts and the licence and exception
                          lists
build_runtime.sh          fetch, build, wire, check and export the prefix
check_boundary.sh         the host boundary gate, on any prefix
drop_unreachable.sh       drops from the shipped tree every object nothing reaches
write_crates.py           the Rust crates a component compiles in: their sources and
                          licence files
write_licenses.py         runtime-licenses.json: the licence files of every component,
                          crate and the Rust standard library
fetch_source.sh           one fetch and one digest rule, shared by the build and the archive
package_sources.sh        the LGPL source archive published beside the packages
smoke_runtime.sh          draw a libadwaita window on ubuntu:22.04 from the built tree
smoke/runtime_smoke.c     the window it draws
smoke/sample.gif          the GIF it decodes, two frames, made for the smoke
write_manifest.py         runtime-manifest.json: the lock plus every shipped file's sha256
compare_manifest.py       what --verify compares
check_options.py          holds every meson flag to the option upstream declares
build_runtime_test.sh     the offline tests; with build_runtime_test_lock.py,
                          build_runtime_test_boundary.sh,
                          build_runtime_test_unreachable.sh,
                          build_runtime_test_crates.py,
                          build_runtime_test_licenses.py and
                          build_runtime_test_compare.py
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
desktop/packaging/runtime/build_runtime.sh --fetch          # fill the source cache, no daemon
desktop/packaging/runtime/package_sources.sh 0.12.2         # the source archive
```

On six cores the build takes about 10 minutes from empty volumes, and about 25
when the image is built too. It resumes: each installed component leaves a
stamp in the build volume, and the volumes carry the cache key they were filled
under, so a changed lock file, patch, Dockerfile, `build_runtime.sh`,
`write_manifest.py`, `drop_unreachable.sh`, `write_crates.py` or
`write_licenses.py` starts from empty and an unchanged set picks up where a
failure stopped.

Outputs land in `$FERMIX_RUNTIME_OUT`, by default
`~/.cache/fermix-desktop-runtime/out`:

| File | What |
|---|---|
| `runtime-<arch>.tar` | The shipped tree, rooted at `usr/lib/fermix-desktop` |
| `runtime-dev-<arch>.tar` | The same tree with headers, `.pc` files and the build-time binaries |
| `runtime-manifest.json` | The lock file plus the sha256 of every shipped file and of the five files above and below it |
| `runtime-licenses.json` | Every component, Rust crate and the Rust standard library, each with its licence and licence files; see below |
| `runtime-licenses.tar.gz` | Those licence files, at the paths the index names |
| `runtime-crates.tar.gz` | The sources of the Rust crates compiled in, as cargo vendored them, for the source archive |
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
files, static libraries, documentation, the MIME database, every build-time
binary, and every library nothing reaches (below). There is no `bin/`; the application ELF's directory is the package's to
create. `libexec/` stays for `gio-launch-desktop` and `gst-plugin-scanner`,
which GLib and GStreamer run by their compiled-in paths. Nothing needs a
post-install step: `loaders.cache`, `giomodule.cache`, `gschemas.compiled` and
the icon cache are generated at build time against the final prefix.
`runtime-manifest.json` ships at
`/usr/share/doc/fermix-desktop/runtime-manifest.json`; its `lock` object carries
each component's `license`. The copyright file is generated from
`runtime-licenses.json` and the texts in `runtime-licenses.tar.gz`; the format
is below. `check_boundary.sh <dir> RUNTIME.lock.json` runs the same gate on any
tree.

If the window comes to link a library of the runtime through a pkg-config
package not in the lock file's `application_packages`, that library may be
pruned from the shipped tree; the package's own NEEDED check then fails, and the
package name goes into the lock file. The lock test reads the window's `-sys`
crates from `desktop/Cargo.lock` and fails first.

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
support, and built with the PNG, JPEG and GIF loaders only (`-Dtiff=disabled
-Dothers=disabled`), plus webp-pixbuf-loader for WebP and librsvg's loader for
SVG. GIF is there because chats send GIFs; its loader is gdk-pixbuf's own and
needs no other library, and the smoke decodes a two-frame GIF
(`smoke/sample.gif`) through it. So nothing is sandboxed, no bubblewrap and no
loader binaries are needed, and the host needs nothing for images.

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
file pins by sha256 and names in librsvg's `cargo.lock`. For a component with a
`cargo` object, `vendor_crates` fetches the crates with `cargo vendor --locked`,
which refuses one whose checksum differs from `Cargo.lock`, into the source
tree; the compile then runs with `CARGO_NET_OFFLINE=true`. It is the only
network fetch the build makes itself. A `Cargo.lock` alone does not trigger it:
fontconfig 2.16's tarball carries one for its optional Fontations backend, which
is not built.

Of the 357, 136 are compiled in: librsvg's three workspace members and 133
from crates.io. `cargo.packages` names the packages librsvg's
meson builds and their features (`librsvg-c` with `pixbuf`, and
`pixbufloader-svg`); the lock test holds them to librsvg's meson options, and
`write_crates.py` holds them to the cargo targets in meson's introspection.
`cargo tree` over those packages, for the host target, without build scripts and
procedural macros, which run at build time and are not linked, gives the crates
compiled in. Their sources go into `runtime-crates.tar.gz`, which the source
archive carries, among them the MPL-2.0 crates whose source has to be offered
with the binary, and their licence files into `runtime-licenses.tar.gz`.

**Crate licences.** Cargo once took `/` between licences to mean OR, and nine
crates compiled in still declare that form: futf (`MIT / Apache-2.0`), fxhash
(`Apache-2.0/MIT`), and language-tags, mac, matrixmultiply, quick-error,
rawpointer, siphasher and tendril (`MIT/Apache-2.0`). `write_crates.py` writes
each as an SPDX expression, the parts trimmed and joined with ` OR `, and
refuses any other licence that is not one: `MIT or Apache-2.0`, `Apache 2.0`, an
operator or a parenthesis with nothing on one side, or `/` beside another
operator. Every licence and exception id has to be on SPDX's lists, which the
lock's `license_list` pins (`json/licenses.json` and `json/exceptions.json` at
the `v3.29.0` tag, by sha256), and none may be deprecated there: `GPL-2.0` or
`GPL-2.0+` is refused. A `LicenseRef-` is the expression's own and on no list;
a crate cannot map one to a text, so `write_licenses.py` refuses a crate that
names one. The lock test holds every component's expression and every standard
text to the same lists, read from the source cache (`build_runtime.sh --fetch`
fills it), and the build holds the crates' to them.

Four of those crates publish no licence file. For exactly those, the lock
file's `cargo.standard_license_texts` names the SPDX ids of their declared
licences, and `write_licenses.py` copies the standard texts to
`standard/<id>.txt`. Each is one entry of `license_text_sources`:
`text/<id>.txt` of SPDX's license-list-data at its `v3.29.0` tag, fetched raw,
pinned by sha256 like a component and never built. The four texts (Apache-2.0,
MIT and MPL-2.0 for crates, Unicode-3.0 for components, below) are 30 KB; the
release tarball they come from is 49 MB. The lock test holds the pinned texts to
exactly the ids the crates and components name. The crates:

| Crate | Declares | Standard texts | Upstream |
|---|---|---|---|
| cssparser-color 0.3.0 | MPL-2.0 | MPL-2.0 | servo/rust-cssparser carries the MPL 2.0 at the commit the crate records (3dd4f636); the published crate leaves it out |
| selectors 0.31.0 | MPL-2.0 | MPL-2.0 | servo/stylo has no licence file at the recorded commit (7f8df16f); its sources point to the MPL 2.0 |
| fxhash 0.2.1 | Apache-2.0 OR MIT | Apache-2.0, MIT | cbreeden/fxhash has no licence file; `lib.rs` is "Copyright 2015 The Rust Project Developers", licensed as Rust is |
| mac 0.1.1 | MIT OR Apache-2.0 | Apache-2.0, MIT | reem/rust-mac has no licence file; its README says "MIT/Apache-2.0, like rust itself" |

`write_crates.py` refuses a crate that ships no licence file and is not in that
list, so a new one fails the build rather than quietly getting a generic text.
It also refuses an entry for a crate that is not compiled in, for one that
ships its own licence file, or naming a licence the crate does not declare.

**Licence files.** `write_licenses.py` reads each component's licence files
from its locked tarball, so the texts carry the copyright holders, which a
generic text would drop. It takes the files at the top of the source tree whose
names start with `COPYING`, `LICENSE`, `LICENCE`, `COPYRIGHT` or `NOTICE`
(libxml2's is `Copyright`), every file in a top-level `LICENSES/` directory
(all of GLib's REUSE texts, below), and what
the component's `extra_license_files` names: texts a top-level file only points
to (FreeType's `docs/FTL.TXT` and `docs/GPLv2.TXT`, libjpeg-turbo's
`README.ijg`), libwebp's `PATENTS`, and the licences of code bundled into what
ships (GTK's roaring and timsort, HarfBuzz's Microsoft USE tables, pcre2's
sljit, the third-party code in webrtc-audio-processing). A licence file that is
a link, as GLib's `COPYING` is, holds the text of its target, which has to be a
licence file taken too. A component with no licence file, a named file its
tarball lacks, a crate with none and no standard text named, and a file in the
tree the index does not name are each refused. Tarball entries that start with
`./`, as AppStream's do, are read like any other. wayland-protocols ships no
file of its own, but GTK compiles code generated from its XML, so its licence
is listed and marked `build_only`.

**Where each licence's text is.** Each `LicenseRef-` an expression names maps,
in the component's `license_refs`, to the one licence file that holds its text,
a path in its source tree (with `.notice` for an excerpt); the lock test holds
it to a file `extra_license_files` or `license_excerpts` names, and
`write_licenses.py` to a file it took. An unmapped `LicenseRef-`, a mapped one
no expression names, and one mapped by two components are refused. The index's
`license_refs` gives each one's path in the licence tree, for the copyright
file's standalone License paragraphs. A licence no file of the component holds
is named in its `standard_license_texts`, and its standard text joins the
component's `license_files` as `standard/<id>.txt`: Unicode-3.0 for Unicode's
data, and the texts the Debian audit below names. Every licence of every
expression has its text among the component's licence files. Three of these
fill gaps older than the audit: GTK's and pango's `COPYING` is the LGPL 2.0,
so the LGPL 2.1 takes its standard text; webp-pixbuf-loader's `LICENSE.LGPL-2`
is the notice, not the licence; and libjpeg-turbo's `LICENSE.md` names the
zlib License of its SIMD code by a link only, so the lock takes the notices of
`simd/jsimd.c` (1-27) and `simd/nasm/jsimdext.inc` (1-28), the code the build
compiles with NASM.

**Notices inside source files.** Some code compiled in carries its licence only
in a comment at the top of the file. The component's `license_excerpts` names
each such file by its path, its first and last line, and the sha256 of those
lines with their line endings; `write_licenses.py` writes the lines to
`<path>.notice` beside the other licence files and ships nothing else of the
file. A file with two notices, as GLib's `gchecksum.c` has, takes two
excerpts, which make one `.notice` in the lock's order; they must come in the
file's order and share no line. A digest that differs, a range past the end of
the file, a file the tarball lacks, a file that is a link and two excerpts of
one file that overlap are each refused, so a release that edits a notice fails
the build until the lock is read and updated.

**Licences of bundled code.** The package's copyright file is generated from
the components' expressions, so each names every licence of code compiled into
what ships, not only the component's main one. Where no SPDX id fits, the
expression names a `LicenseRef-` and the component's licence files carry its
text. The lock test pins each of these, and every expression the Debian audit
below changed. That audit added licences to glib, gtk, harfbuzz and
webrtc-audio-processing too; the expressions here are the final ones, and its
table gives the evidence for what it added:

| Component | Expression | Evidence | Texts |
|---|---|---|---|
| glib | `LGPL-2.1-or-later AND (LGPL-2.1-or-later OR AFL-2.0) AND LGPL-2.0-or-later AND MIT AND bzip2-1.0.6 AND LicenseRef-glib-gbsearcharray AND LicenseRef-glib-gchecksum AND Unicode-3.0` | See GLib's licence, below | `LICENSES/`, the four `.notice` files, `LicenseRef-glib-gbsearcharray` in `glib/gbsearcharray.h.notice` and `LicenseRef-glib-gchecksum` in `glib/gchecksum.c.notice` |
| gtk | `LGPL-2.1-or-later AND LGPL-2.0-or-later AND Apache-2.0 AND MIT AND MIT-open-group AND HPND-sell-variant AND TCL AND CC0-1.0 AND Unicode-3.0` | `gtk/roaring/roaring.c` (CRoaring 4.3.5) says `SPDX-License-Identifier: Apache-2.0`, and `gtk/gtkbitset.c` includes it whole; `gtk/timsort/gtktimsort-impl.c` is Apache-2.0 (Benjamin Otte, Patrick O. Perry, the Android Open Source Project) | `gtk/roaring/COPYING`, `gtk/timsort/COPYING`, both the Apache-2.0 text |
| harfbuzz | `MIT-Modern-Variant AND MIT AND ISC AND Unicode-3.0` | `src/hb-ot-shaper-use-table.hh`, compiled in, is generated from Microsoft's `IndicSyllabicCategory-Additional.txt` and `IndicPositionalCategory-Additional.txt`, which `src/ms-use/COPYING` licenses MIT, Copyright (c) Microsoft Corporation | `src/ms-use/COPYING` |
| pcre2 | `BSD-3-Clause WITH PCRE2-exception AND BSD-2-Clause AND Unicode-3.0` | The lock builds with `--enable-jit`; `LICENCE.md` says the JIT compiler is "separately licensed under the 2-clause BSD licence", and `deps/sljit/LICENSE` is that licence (Zoltan Herczeg) | `deps/sljit/LICENSE` |
| webrtc-audio-processing | `BSD-3-Clause AND BSD-2-Clause AND LicenseRef-webrtc-ooura AND LicenseRef-webrtc-spl-sqrt-floor AND LicenseRef-webrtc-pffft` | Four `third_party` directories are compiled into `libwebrtc-audio-processing-2.so.1`: ooura and spl_sqrt_floor among `common_audio`'s sources, pffft and rnnoise as static libraries the library's `dependencies` name. rnnoise is BSD-3-Clause. The other three have notices no SPDX id matches: ooura (Takuya OOURA's), spl_sqrt_floor (Wilco Dijkstra's public-domain grant) and pffft (the FFTPACK licence of Julien Pommier and UCAR, which differs from SPDX's `UCAR`). The fifth, `modules/third_party/fft` (Mark Olesen's notice), is built as a static library that nothing links: its `fft_dep` is declared and never used, and the 2,344 symbols the shipped library exports, which include ooura's, spl_sqrt_floor's, pffft's and rnnoise's, include none of its `WebRtcIsac_Fft*` | `webrtc/LICENSE` and `PATENTS`; the `LICENSE` in `common_audio/third_party/ooura`, `common_audio/third_party/spl_sqrt_floor` and `third_party/pffft`, which `license_refs` maps to the three `LicenseRef`s in that order; `third_party/rnnoise/COPYING` |

| fontconfig | `HPND-sell-variant AND MIT-Modern-Variant AND MIT AND LicenseRef-fontconfig-fcmd5 AND LicenseRef-fontconfig-ftglue AND Unicode-3.0` | `COPYING` gives five notices beyond Keith Packard's, each for a file compiled into libfontconfig: `src/fcatomic.h` and `src/fcmutex.h` (MIT-Modern-Variant, from HarfBuzz), `#include`d by `src/fcint.h:53-54`, which every source includes, with `fcatomic.c` at `src/meson.build:2`; `src/fcfoundry.h` (MIT, Juliusz Chroboczek), `#include`d at `src/fcfreetype.c:48` (`src/meson.build:11`); `src/fcmd5.h` (Colin Plumb's MD5, public domain), `#include`d at `src/fccache.c:25` (`src/meson.build:3`); and `src/ftglue.c` (David Turner's glue, public domain) at `src/meson.build:29`. No SPDX id is a public-domain dedication, so each of the two takes a `LicenseRef-` | `COPYING` holds all five texts; `license_refs` maps the two `LicenseRef`s to the excerpts `src/fcmd5.h` 1-16 and `src/ftglue.c` 1-9 |

Unicode-3.0 in these and four more is for Unicode's data, below.

**GLib's licence.** Its `LICENSES/` holds eleven texts. Two of them apply to
code in the shipped libraries, `LGPL-2.1-or-later` and, for `gio/xdgmime`,
`LGPL-2.1-or-later OR AFL-2.0`. By the SPDX headers and `.reuse/dep5` of 2.88.3, the others
cover what does not ship: Apache-2.0 with LLVM-exception for `fuzzing/` and CI,
MIT for `tests/lib`, GPL-2.0-or-later for `glib/tests` and `tools`, CC0-1.0 and
CC-BY-SA-3.0 for documentation and test data, LicenseRef-old-glib-tests for
tests, and LGPL-2.1-only OR MPL-1.1 for `girepository/cmph`, whose library is
dropped from the shipped tree (above). `glib/libcharset/localcharset.c`, built
into `charset_lib` (`glib/meson.build:31`) and linked into libglib (`:432`), is
"GNU Library General Public License ... version 2, or (at your option) any
later version", so the expression names LGPL-2.0-or-later, with its standard
text, which `LICENSES/` lacks. Four files compiled in carry notices of their
own and no SPDX header, and the lock names each as an excerpt:

| File | Lines | Licence | Compiled in by |
|---|---|---|---|
| `glib/gutilsprivate.c` | 1-24 | `MIT`, Red Hat 2007 | `#include "gutilsprivate.c"` at `glib/gutils.c:2250` |
| `glib/valgrind.h` | 1-56 | `bzip2-1.0.6`, Julian Seward; SPDX's entry for that licence lists a valgrind file among its sources | `glib/grcbox.c`, `glib/garcbox.c`, and `glib/gvalgrind.h`, which defines `ENABLE_VALGRIND` on every compiler but MSVC and is included by `gerror.c`, `ghash.c`, `gutf8.c`, `gobject/gatomicarray.c`, `gclosure.c` and `gtype.c` |
| `glib/gbsearcharray.h` | 1-18 | Tim Janik's permissive notice, which no SPDX id matches: `LicenseRef-glib-gbsearcharray` | `gobject/gsignal.c` and `gobject/gvalue.c` |
| `glib/gchecksum.c` | 205-217 and 477-488 | Colin Plumb's public-domain MD5 notice and A.M. Kuchling's "Distribute and use freely" SHA-1 notice (Debian's Plumb-PD and Kuchling-PD), which no SPDX id matches: `LicenseRef-glib-gchecksum`, the two excerpts in one notice | `glib/meson.build:277` |

With its Unicode tables (below), the expression is therefore
`LGPL-2.1-or-later AND (LGPL-2.1-or-later OR AFL-2.0) AND LGPL-2.0-or-later AND
MIT AND bzip2-1.0.6 AND LicenseRef-glib-gbsearcharray AND
LicenseRef-glib-gchecksum AND Unicode-3.0`.

**Unicode's data.** Eight components compile in tables generated from
Unicode's data files, or ship data taken from them, and their expressions name
Unicode-3.0, as `rust_std`'s does for core's tables: its terms ask for the
notice with the data or in its documentation. unicode.org's terms put every
Unicode Data File under the Unicode License v3 unless the file says otherwise
at its release, and none of these does, so it is Unicode-3.0 whatever the
version. No file of these components holds that licence's text, so each names
Unicode-3.0 among its `standard_license_texts`. A scan of every locked tarball for
the UCD's file names found no other: FreeType's matches are URLs in its
documentation and headers, and the rest are tests, documentation and
generator scripts that do not ship.

| Component | Tables from Unicode's data | Compiled in or shipped by |
|---|---|---|
| glib 2.88.3 | `glib/gunichartables.h`, `gscripttable.h`, `gunibreak.h`, `gunidecomp.h` and `gunicomp.h` (`gen-unicode-tables.pl`, Unicode 17.0.0), and `gmirroringtable.h` (BidiMirroring.txt) | `#include` at `glib/guniprop.c:34-36`, `gunibreak.c:25` and `gunidecomp.c:27,30`, all in `glib/meson.build:336-340` |
| harfbuzz 14.4.0 | `src/hb-ucd-table.hh` (the UCD in XML), `hb-unicode-emoji-table.hh` (emoji-data.txt), and the Arabic, Arabic joining, Indic, USE and vowel-constraint tables, all Unicode 17.0.0 | `hb-ucd.cc:21` (`src/meson.build:271`; `hb_unicode_funcs_get_default` returns the UCD functions, `hb-unicode.cc:155`), `hb-unicode.cc:618`, `hb-ot-shaper-arabic.cc:84`, `hb-ot-shaper-use.cc:34,36`, and `hb-ot-shaper-indic-table.cc` and `hb-ot-shaper-vowel-constraints.cc` themselves (`src/meson.build:221,231`) |
| pcre2 10.48 | `src/pcre2_ucd.c`, "auto-generated from Unicode data files" (Unicode 17.0.0) | `COMMON_SOURCES` of `libpcre2_8_la_SOURCES`, `Makefile.am:464,473`; the shipped `libpcre2-8.so.0` carries its version string `17.0.0` |
| fribidi 1.0.16 | The six `*.tab.i` tables, generated at build time from the tarball's `gen.tab/unidata/` (UnicodeData.txt, ArabicShaping.txt, BidiMirroring.txt, BidiBrackets.txt, Unicode 16.0.0) | `gen.tab/meson.build:50-80`; `lib/meson.build:74-76` compiles them into libfribidi, `#include`d at `fribidi-bidi-types.c:42`, `fribidi-joining-types.c:42`, `fribidi-arabic.c:52`, `fribidi-mirroring.c:35` and `fribidi-brackets.c:36-37` |
| fontconfig 2.16.0 | `fccase.h`, generated at build time from the tarball's `fc-case/CaseFolding.txt` (15.1.0); and `fclang.h`, which takes `fc-lang/und_zsye.orth` (from emoji-data.txt 5.0) and `und_zmth.orth` (from MathClass-9.txt) | `fc-case/meson.build:1-4`, `src/meson.build:55`, `#include` at `src/fcstr.c:87`; `fc-lang/meson.build:247-248`, `#include` at `src/fclang.c:40` |
| pango 1.56.4 | `pango/pango-break-table.h` (`tools/gen-break-table.py`, Unicode 16.0.0), `pango-emoji-table.h` (emoji-data.txt 16.0), and the upright table in `itemize.c` (VerticalOrientation.txt 11.0.0) | `#include` at `break.c:28` and `pango-emoji.c:54`; `itemize.c:147-152`; `pango/meson.build:2,6,13` |
| libxml2 2.13.9 | `xmlunicode.c`, generated by `genUnicode.py` from Blocks-4.0.1.txt and UnicodeData-4.0.1.txt | Compiled with regexps, which the lock leaves on; the shipped `libxml2.so.2` exports its `xmlUCSIsCatL` and `xmlUCSIsBlock` |
| gtk 4.22.5 | The 23 emoji bundles, `share/gtk-4.0/emoji/<lang>.gresource`, made from `gtk/emoji/<lang>.data`: the emoji and their names from Unicode and CLDR, through emojibase (`gtk/emoji/README.md`); and `gtkimcontextsimpleseqs.h`, whose inputs include UnicodeData.txt | `gtk/meson.build:790-800` installs the bundles, and the shipped tree carries all 23; `#include` at `gtk/gtkimcontextsimple.c:34` |

**Debian's copyright files.** Each expression names every licence of code
compiled into what ships. Debian's copyright file sets the floor: a licence it
records for a file the runtime compiles in or ships is in the component's
expression, and it decides the unclear cases (the known residuals, below). Every component's
unstable copyright file, at
`https://metadata.ftp-master.debian.org/changelogs/main/<prefix>/<package>/unstable_copyright`,
was read on 2026-10-06 at 02:10 UTC; Fedora's spec was never needed, since
Debian packages all 35. For each licence Debian names that the expression
lacked, the files it covers were looked up in the locked tarball, not
Debian's, and followed to what ships: an option the lock sets, a platform
directory, demos, tests, tools or documentation, or the compile line or
`#include` that brings the file in.

- A licence whose files ship joined the expression, with its text from the
  component's own tree (an excerpt, or a file) where the tree carries one, and
  otherwise the standard text from license-list-data 3.29.0.
- A licence whose files do not ship has its reason in the table.
- Where Debian offers a choice (`MIT or LGPL-2+`), an alternative already in
  the expression covers it.
- Where Debian's version differs from the lock's, the version is in the row,
  and the reason holds for the lock's tarball.
- Debian's fontconfig, cairo and adwaita-icon-theme files are not
  machine-readable, so they were read by hand.

"None" in the added column means every licence Debian names was already in the
expression or is ruled out beside it.

| Component | Debian unstable (read 2026-10-06 02:10 UTC) | Added, and what ships it | Debian's other licences, and why they are not added |
|---|---|---|---|
| libffi 3.8.0 | [libffi 3.8.0-2](https://metadata.ftp-master.debian.org/changelogs/main/libf/libffi/unstable_copyright) | `CC0-1.0`: `src/dlmalloc.c`, Doug Lea's malloc "released to the public domain, as explained at" CC0 1.0 (Debian: public-domain), is `#include`d at `src/closures.c:570` in the `FFI_MMAP_EXEC_WRIT` branch, which `closures.c:135,142` defines on Linux. Excerpt 1-5 and the standard text | `GPL-2.0-or-later`: `libtool-ldflags` and the testsuite's `alignof.h`; `MPL-1.1 or GPL-2+ or LGPL-2.1+`: `msvcc.sh`, the MSVC wrapper; `GPL-3.0-or-later`: `testsuite/` and `make_sunver.pl`; `X11`: `install-sh`. Build tools and tests |
| pcre2 10.48 | [pcre2 10.48-3.1](https://metadata.ftp-master.debian.org/changelogs/main/p/pcre2/unstable_copyright) | None | Public domain: `testdata/`; `X11`: `install-sh` |
| expat 2.8.4 | [expat 2.8.5-2](https://metadata.ftp-master.debian.org/changelogs/main/e/expat/unstable_copyright) | None | None |
| libpng 1.6.58 | [libpng1.6 1.6.59-1](https://metadata.ftp-master.debian.org/changelogs/main/libp/libpng1.6/unstable_copyright) | None | `Apache-2.0`: `contrib/oss-fuzz/Dockerfile`; `BSD-4-Clause or GPL-2+`: `contrib/gregbook` and `contrib/visupng`; `MIT`: `ci/`, `contrib/pngexif`, `contrib/pngminus` and build scripts. The lock builds the library alone (`--disable-tools`), and nothing under `contrib/` or `ci/` is built |
| libjpeg-turbo 3.2.0 | [libjpeg-turbo 1:3.1.3-4](https://metadata.ftp-master.debian.org/changelogs/main/libj/libjpeg-turbo/unstable_copyright) | None. Zlib was already named; its text is new (above) | `NTP`: `src/rdcolmap.c`, `rdgif.c`, `rdppm.c` and `wrgif.c` go into cjpeg and djpeg (`sharedlib/CMakeLists.txt:94-113`) and TurboJPEG (`-DWITH_TURBOJPEG=OFF`); `libjpeg.so.62` is `JPEG_SOURCES` (`CMakeLists.txt:653`), which has none of them, and no program ships. `MIT`: the documentation's JavaScript |
| libtiff 4.7.2 | [tiff 4.7.2-1](https://metadata.ftp-master.debian.org/changelogs/main/t/tiff/unstable_copyright) | None | None; Debian's Hylafax stanza is the libtiff licence |
| libwebp 1.6.0 | [libwebp 1.6.0-0.1](https://metadata.ftp-master.debian.org/changelogs/main/libw/libwebp/unstable_copyright) | None | `Apache-2.0`: tests |
| freetype 2.13.3 | [freetype 2.14.3+dfsg-3](https://metadata.ftp-master.debian.org/changelogs/main/f/freetype/unstable_copyright) | `MIT`: the BDF and PCF drivers, on at `modules.cfg:65,68`, which meson reads (`meson.build:61-80`), and `src/base/fthash.c`, `#include`d at `src/base/ftbase.c:28`. Excerpts: `src/bdf/README` 99-144, `src/pcf/README` 66-88, `fthash.c` 9-31. `MIT-open-group` (Debian: OpenGroup-MIT): `src/pcf/pcfutil.c`, excerpt 1-25 | `Zlib`: `src/gzip/`, replaced by the system zlib: `-Dzlib=system` sets `FT_CONFIG_OPTION_SYSTEM_ZLIB` (`meson.build:276-315`), and `ftgzip.c:43-45` then includes `<zlib.h>`. `BSL-1.0`: `src/dlg/`, which meson does not compile; only `FT_DEBUG_LOGGING` uses it. Public domain: `src/base/md5.c`, `#include`d at `ftobjs.c:77` only under `FT_DEBUG_LEVEL_TRACE` (`ftobjs.c:52-99`), which the release build leaves off. `BSD-3-Clause`, `FSFAP` and `GPL-3.0-or-later`: build files |
| fribidi 1.0.16 | [fribidi 1.0.16-5](https://metadata.ftp-master.debian.org/changelogs/main/f/fribidi/unstable_copyright) | None | None |
| pixman 0.46.4 | [pixman 0.46.4-1](https://metadata.ftp-master.debian.org/changelogs/main/p/pixman/unstable_copyright) | None | None |
| fontconfig 2.16.0 | [fontconfig 2.17.1-5](https://metadata.ftp-master.debian.org/changelogs/main/f/fontconfig/unstable_copyright), by hand | None in this pass; the five notices of `COPYING` are named above | None; the MacRoman table is a known residual (below) |
| glib 2.88.3 | [glib2.0 2.90.0-1](https://metadata.ftp-master.debian.org/changelogs/main/g/glib2.0/unstable_copyright) | `LGPL-2.0-or-later` for `glib/libcharset/localcharset.c`, and `LicenseRef-glib-gchecksum` for `glib/gchecksum.c` (Debian: Plumb-PD and Kuchling-PD); see GLib's licence, above | `Apache-2.0 with LLVM exception`: `fuzzing/` and CI; `CC-BY-SA-3.0`: documentation; `CC0-1.0`: tests and `.gitignore` files; `CC0-1.0` and Mingw's public domain: `glib/dirent/`, built for MSVC alone (`glib/meson.build:378-380`); `FSFULLR`: build files; `GPL-2.0-or-later`: documentation, tests and tools; gnulib's printf: built only without a usable system printf (`meson.build:1384-1388`), and the built `glibconfig.h` defines `GLIB_USING_SYSTEM_PRINTF`; cmph: girepository, dropped from the shipped tree; `LGPL-3.0-or-later` and the old GLib tests' licence: tests; `win_iconv.c`: Windows only |
| harfbuzz 14.4.0 | [harfbuzz 12.3.2-2](https://metadata.ftp-master.debian.org/changelogs/main/h/harfbuzz/unstable_copyright) | `ISC`: `src/hb-ucd.cc` (Grigori Goronzy), `src/meson.build:271`. Excerpt 1-15 | Monotype's, `Apache-2.0`, `CC0-1.0`, `GPL-2+ with Font exception`, `GPL-3.0-or-later`, `OFL-1.1` and `UFL-1.0`: test and benchmark fonts |
| cairo 1.18.4 | [cairo 1.18.4-3](https://metadata.ftp-master.debian.org/changelogs/main/c/cairo/unstable_copyright), by hand | `HPND-sell-variant`: Debian quotes David Reveman's notice, which `src/cairo-pattern.c` carries (`src/meson.build:68`). Excerpt 3-29 | None. Debian's other text is `COPYING`'s: every source file under the LGPL 2.1 or the MPL 1.1, as the expression says |
| graphene 1.10.8 | [graphene 1.10.8-5](https://metadata.ftp-master.debian.org/changelogs/main/g/graphene/unstable_copyright) | None | None |
| pango 1.56.4 | [pango1.0 1.58.2-1](https://metadata.ftp-master.debian.org/changelogs/main/p/pango1.0/unstable_copyright) | `LGPL-2.1-or-later`: `pango/json/gtkjsonparser.c` and `gtkjsonprinter.c` (`pango/meson.build:32-33`); the standard text, since `COPYING` is the LGPL 2.0. `BSD-3-Clause` (Debian: Chromium-BSD-style): `pango/emoji_presentation_scanner.c`, `#include`d at `pango-emoji.c:209` (`meson.build:13`); its header points to a LICENSE file the tarball lacks, so excerpt 3-5 and the standard text. `ICU`: `pango/pango-script.c` (`meson.build:26`), excerpt 21-54. `TCL`: the Tk routines in `pango/pango-color.c` (`meson.build:10`), excerpt 105-147 | `Example`: `examples/`; `Apache-2.0`, `Bitstream-Vera` and `OFL-1.1`: test fonts |
| gdk-pixbuf 2.42.12 | [gdk-pixbuf 2.44.8+dfsg-1](https://metadata.ftp-master.debian.org/changelogs/main/g/gdk-pixbuf/unstable_copyright) | `LGPL-2.0-or-later`: library sources such as `gdk-pixbuf/gdk-pixbuf-scale.c` (`gdk-pixbuf/meson.build:113`) and the GIF loader's `lzw.c` and `io-gif-animation.c` (`meson.build:18`) say "version 2 of the License, or (at your option) any later version". The standard text, since `COPYING` is the LGPL 2.1 | `CC0-1.0`, in Debian's `Files: *`: in the tarball, only `docs/*.toml.in` carry it; `GPL-2.0-or-later`: `thumbnailer/`, whose program does not ship, and tests |
| webp-pixbuf-loader 0.2.7 | [webp-pixbuf-loader 0.2.7-3](https://metadata.ftp-master.debian.org/changelogs/main/w/webp-pixbuf-loader/unstable_copyright) | None. Its text is new (above) | None |
| libxml2 2.13.9 | [libxml2 2.15.4+dfsg-1](https://metadata.ftp-master.debian.org/changelogs/main/libx/libxml2/unstable_copyright) | `ISC`: `dict.c` and `list.c`, both in `libxml2_la_SOURCES` (`Makefile.am:45-46`). Excerpts 1-17 and 1-16 | None; `timsort.h` is MIT, which the expression names |
| librsvg 2.62.4 | [librsvg 2.63.2+dfsg-1](https://metadata.ftp-master.debian.org/changelogs/main/libr/librsvg/unstable_copyright) | None | `LGPL-2+`, Debian's `Files: *`: no file of the 2.62.4 tarball carries a version 2 notice; the C headers and the pixbuf loader say "version 2.1", and `Cargo.toml` declares `LGPL-2.1-or-later`. `BSD-3-clause`, `CC-zero-waive-1.0-us`, `Expat` and `OFL-1.1`: test fixtures. `debian/missing-sources/`: Debian's own; the crates compiled in are the index's `crates` |
| libxmlb 0.3.29 | [libxmlb 0.3.29-1](https://metadata.ftp-master.debian.org/changelogs/main/libx/libxmlb/unstable_copyright) | None | `CC0-1.0`: `data/fuzzing-src/appdata.xml`, fuzzing input |
| appstream 1.0.6 | [appstream 1.2.1-1](https://metadata.ftp-master.debian.org/changelogs/main/a/appstream/unstable_copyright) | `FSFAP`: `data/platforms.yml` generates `data/platform_arch.txt`, `platform_os.txt` and `platform_env.txt`, which say so on their first line; `src/appstream.gresource.xml` compiles them into libappstream (`src/meson.build:157`). The standard text | `FSFAP` for `data/its/`, installed to `share/gettext/its`, which the runtime does not ship, and for the two tool metainfo files and a test sample; `CC-BY-SA-4.0 or GFDL-1.3+` and `GPL-2+ or GPL-3`: documentation and its stylesheets; `CC0-1.0` and `OFL-1.1`: test samples |
| wayland 1.26.0 | [wayland 1.26.0-1](https://metadata.ftp-master.debian.org/changelogs/main/w/wayland/unstable_copyright) | None | None |
| wayland-protocols 1.49 | [wayland-protocols 1.49-1](https://metadata.ftp-master.debian.org/changelogs/main/w/wayland-protocols/unstable_copyright) | `HPND-sell-variant`: GTK compiles wayland-scanner's code for `pointer-gestures-unstable-v1.xml` (`gdk/wayland/meson.build:68`) and `text-input-unstable-v3.xml` (`gtk/meson.build:680`), and the scanner copies each `<copyright>` into that code. Excerpts of both | `HPND-sell-variant` for `ext-data-control-v1`, `ext-foreign-toplevel-list-v1`, `ext-workspace-v1` and `xx-text-input-v3`, which GTK 4.22 does not use |
| libepoxy 1.5.10 | [libepoxy 1.5.10-2](https://metadata.ftp-master.debian.org/changelogs/main/libe/libepoxy/unstable_copyright) | None | None |
| gtk 4.22.5 | [gtk4 4.24.1+ds-1](https://metadata.ftp-master.debian.org/changelogs/main/g/gtk4/unstable_copyright) | Debian's `Files: *` names ten licences at once, so the 4.22.5 tarball was searched for each one's text. `LGPL-2.0-or-later`: 492 compiled sources, `gsk/gskrendernode.c` among them, say "version 2 of the License", and `gtk/gtksecurememory.c` is `SPDX-License-Identifier: LGPL-2.0-or-later`; `COPYING` is that text. `MIT` (Debian: Expat): the inspector (`gtk/meson.build:3`), with excerpts of `gtk/inspector/window.c`, `css-node-tree.c` and `logs.c` for its three sets of holders, and `gdk/wayland/protocol/xx-session-management-v1.xml` (`gdk/wayland/meson.build:174`). `HPND-sell-variant` (Debian: X11R5-permissive): `gdk/x11/xsettings-client.c` (Red Hat) and `gdkxftdefaults.c` (Keith Packard), `gdk/x11/meson.build:21,39`. `MIT-open-group`: `gdk/x11/gdkasync.c` (The Open Group), `gdk/x11/meson.build:27`; the lock builds the X11 backend. `TCL` (Debian: sun-permissive): the Tk text widget's files, `gtk/gtktextbtree.c` and its kin; excerpt 1-53. `CC0-1.0`: the symbolic icons in `gtk/icons/`, which `gtk/gen-gtk-gresources-xml.py:69-70` puts in libgtk's resources; the standard text | `lcs-telegraphics-permissive`: `gdk/win32/pktdef.h` and `wintab.h`; `ZPL-2.1`: `gdk/win32/winpointer.h`; both Windows only (`-Dwin32-backend=false`). `BSD-3-clause-Google`: no file of the 4.22.5 tarball carries it. `Apache-2.0 with LLVM exception`: `.gitlab-ci/clang-format-diff.py`. `CC0-1.0` for the demos' metainfo: demos. `GPL-3+`: `debian/tests`, and `Unicode-DFS-2016`: `debian/missing-sources`, both Debian's own |
| libadwaita 1.9.4 | [libadwaita-1 1.10.0-2](https://metadata.ftp-master.debian.org/changelogs/main/liba/libadwaita-1/unstable_copyright) | None | `CC-BY-SA-4.0` and `CC0-1.0`: `demo/` and `tests/` (`-Dexamples=false`, `-Dtests=false`) |
| dconf 0.40.0 | [dconf 51.0-2](https://metadata.ftp-master.debian.org/changelogs/main/d/dconf/unstable_copyright) | `LGPL-2.0-or-later`: the GSettings module's `gsettings/dconfsettingsbackend.c` and the `common/`, `engine/`, `gdbus/` and `shm/` code it links say "version 2 of the licence, or (at your option) any later version". The standard text | None |
| adwaita-icon-theme 50.0 | [adwaita-icon-theme 51.0-1](https://metadata.ftp-master.debian.org/changelogs/main/a/adwaita-icon-theme/unstable_copyright), by hand | `CC-BY-SA-4.0`: `Adwaita/symbolic/legacy/preferences-system-parental-controls-symbolic.svg` says so in its metadata, and Debian puts the `pan-*-symbolic` and `night-light-symbolic` icons under it; all of them ship. The standard text | `CC0-1.0`: `folder-projects-symbolic.svg`, which 50.0 lacks; `GFDL-1.2+ or CC-BY-SA-3.0 or CC-BY-SA-2.0-IT, and CC-BY-3.0-US`: `src/fullcolor/accessories-dictionary.svg`, a source `meson.build` does not install (it installs `Adwaita/`); `GPL-2` and `GPL-3+`: `src/cursors/*.py`, tools; `GPL-unspecified` and `CC-BY-SA-3.0-US or LGPL-3`: `po/`, and no catalogue of it ships |
| abseil-cpp 20240722.2 | [abseil 20260526.0-2](https://metadata.ftp-master.debian.org/changelogs/main/a/abseil/unstable_copyright) | None | None |
| webrtc-audio-processing 2.1 | [webrtc-audio-processing 1.3-3](https://metadata.ftp-master.debian.org/changelogs/main/w/webrtc-audio-processing/unstable_copyright) | `BSD-2-Clause`: `webrtc/third_party/rnnoise/src/rnn_activations.h` (Octasic, Jean-Marc Valin), `#include`d by `agc2/rnn_vad/rnn_fc.cc` and `rnn_gru.cc` (`webrtc/modules/audio_processing/meson.build:91-92`). Excerpt 1-26 | `Custom-FFT`: `modules/third_party/fft/fft.c`, built into a library nothing links (above) |
| gstreamer 1.26.11 | [gstreamer1.0 1.28.7-1](https://metadata.ftp-master.debian.org/changelogs/main/g/gstreamer1.0/unstable_copyright) | `LGPL-2.0-or-later`: nearly every source of libgstreamer, libgstbase, coreelements and gst-plugin-scanner, `gst/gst.c` among them, says "version 2 of the License". The standard text, since `COPYING` is the LGPL 2.1. `BSD-3-Clause`, which Debian does not record (it puts `gst/gsturi.c` under LGPL-2+ alone): `gst/gsturi.c:61-96` compiles FreeBSD's `strcasestr` (the Regents of the University of California) under `#ifndef HAVE_STRCASESTR`, and gstreamer's `meson.build` never defines `HAVE_STRCASESTR` (its `check_functions`, `meson.build:257-275`, does not probe for it), so the fallback is in libgstreamer. Excerpt 64-96 | `MPL-2.0`: `libs/gst/helpers/ptp/`, the PTP helper, which `-Dauto_features=disabled` leaves out; `GFDL` and public domain: `docs/` and `po/`, and no GStreamer catalogue ships |
| gst-plugins-base 1.26.11 | [gst-plugins-base1.0 1.28.7-1](https://metadata.ftp-master.debian.org/changelogs/main/g/gst-plugins-base1.0/unstable_copyright) | `LGPL-2.0-or-later`: the sources of libgstaudio, libgsttag and libgstapp, and of the app, audioconvert, audioresample and audiotestsrc plugins. The standard text | `BSD-3-clause`: `gst-libs/gst/fft/` (libgstfft, dropped from the shipped tree) and `ext/ogg/` (not built); `MIT or LGPL-2+`: `gst-libs/gst/rtsp/` and `sdp/`, dropped; public domain: `po/` |
| gst-plugins-good 1.26.11 | [gst-plugins-good1.0 1.28.7-1](https://metadata.ftp-master.debian.org/changelogs/main/g/gst-plugins-good1.0/unstable_copyright) | `LGPL-2.0-or-later`: `gst/level/` and `ext/pulse/`. The standard text | `BSD-3-clause`: `gst/monoscope/`, `sys/oss4/`; `LGPL-2`: `gst/rtp/gstrtpmp4adepay.*`; `MIT or LGPL-2+`: `gst/isomp4/`, `gst/rtsp/`, `ext/jack/`, `sys/osxaudio/`. None is built: the lock enables `pulse` and `level` alone |
| gst-plugins-bad 1.26.11 | [gst-plugins-bad1.0 1.28.7-2](https://metadata.ftp-master.debian.org/changelogs/main/g/gst-plugins-bad1.0/unstable_copyright) | `LGPL-2.0-or-later`: `gst-libs/gst/audio/`, libgstbadaudio. The standard text | `GPL-2+`: `gst/audiovisualizers/`, `gst/mpegpsmux/`, `sys/dvb/`; `ISC`: `sys/decklink/`; `MIT`: `gst/librfb/`, `sys/shm/`; `MPL-1.1`, alone or as a choice: `ext/resindvd/`, `gst/mpegdemux/`, `gst/mpegpsmux/`, `gst/mpegtsmux/`; `MIT or LGPL-2+`: `ext/openal/`, `ext/opencv/` and other plugins; `GPL-3+`: a test. None is built: the lock enables `webrtcdsp` alone, and libgstbadaudio and that plugin are what ship |

**Known residuals.** Four pieces of third-party code or data compiled into what
ships come under terms no expression names. Each is data whose terms are
unclear, or code taken from public-domain code, which asks for no notice. The
bar for these is Debian's copyright file for the same package: where it
records one, so does the runtime. It records none of the four, so the runtime
adds no licence for them and lists them here. Code under a clear licence is
named whether Debian records it or not, as gstreamer's BSD-3-Clause is above.
Debian's files were read from unstable on 2026-10-06, at 02:04 UTC for gtk4
4.24.1+ds-1 and fontconfig 2.17.1-5, and at 02:10 UTC for gst-plugins-base1.0
1.28.7-1.

| What | Where it is compiled in | Debian |
|---|---|---|
| GTK's emoji data: emoji names and keywords from emojibase (MIT, Miles Johnson), which takes them from Unicode and CLDR | The 23 bundles `share/gtk-4.0/emoji/<lang>.gresource`, made from `gtk/emoji/<lang>.data` by `gtk/meson.build:790-800`; `gtk/emoji/README.md` names emojibase | Debian's gtk4 copyright does not record it either: no stanza names `gtk/emoji/*.data` or emojibase, and its `gtk/icons/emoji-*` stanza is the CC0-1.0 icons. <https://metadata.ftp-master.debian.org/changelogs/main/g/gtk4/unstable_copyright> |
| GTK's compose tables: a precompiled copy of libX11's `nls/en_US.UTF-8/Compose.pre` (X.Org) | `gtk/compose/sequences-*-endian` and `chars`, bundled into libgtk's resources by `gtk/gen-gtk-gresources-xml.py:81-82` and read at `gtk/gtkimcontextsimple.c:111,115`; and `gtk/gtkimcontextsimpleseqs.h`, `#include`d at `gtkimcontextsimple.c:34` | Debian's gtk4 copyright does not record it either: no stanza names `gtk/compose/`, and its one X notice, The Open Group's (1986-1998, X11R5-permissive, in `Files: *`), matches `gdk/x11/gdkasync.c`, the only file in the 4.22.5 tarball that carries it. Same URL |
| fontconfig's MacRoman table, "From http://www.unicode.org/Public/MAPPINGS/VENDORS/APPLE/ROMAN.TXT", Apple's mapping file under Apple's terms | `fcMacRomanNonASCIIToUnicode` at `src/fcfreetype.c:506`, `src/meson.build:11` | Debian's fontconfig copyright does not record it either: it gives Keith Packard's notice alone. <https://metadata.ftp-master.debian.org/changelogs/main/f/fontconfig/unstable_copyright> |
| audiotestsrc's pink noise, "based on" Phil Burk's `patest_pink.c`, "released under public domain", which asks for no notice | `gst/audiotestsrc/gstaudiotestsrc.c:745-749`, in the shipped audiotestsrc plugin | Debian's gst-plugins-base1.0 copyright puts the file under LGPL-2+ alone. <https://metadata.ftp-master.debian.org/changelogs/main/g/gst-plugins-base1.0/unstable_copyright> |

**Voice.** The window's pipelines (`desktop/app/src/audio.rs`) make
`pulsesrc`, `pulsesink`, `webrtcdsp`, `webrtcechoprobe`, `audioconvert`,
`audioresample`, `capsfilter`, `audiotestsrc`, `fakesink`, `appsrc` and
`appsink`. Each GStreamer module is configured with `-Dauto_features=disabled`
and enables only the plugins that provide those: `tools` in core (for
`gst-inspect-1.0`), `app`, `audioconvert`, `audioresample` and `audiotestsrc`
in base, `pulse` and `level` in good, `webrtcdsp` in bad. The lock file test
refuses any other enable. `level` is built because the plan names it;
`audio.rs` computes its own meter today. The libraries each module always builds
(gst-plugins-bad's `gst-libs`, for one) are built, and the ones nothing reaches
are dropped from the shipped tree.

**Only what is reached ships.** `drop_unreachable.sh` walks the NEEDED graph of
the shipped tree from its roots: the libraries the window links, which are the
`-l` flags of the lock file's `application_packages` (the pkg-config package of
every `-sys` crate in `desktop/Cargo.lock`), the plugin directories compiled
into GStreamer, gdk-pixbuf and GIO, and `libexec/`. Every ELF object it does not
reach is removed, with the links that name it. A library loaded with `dlopen`
by name appears in no NEEDED entry, so the script first searches every file
that stays for the soname of each object it would drop, and stops, removing
nothing, if one names it. The boundary gate then runs again over the pruned
tree. What goes: the gst-plugins-base and gst-plugins-bad libraries no plugin
the window uses links (codecparsers, video, pbutils, rtp, rtsp, sdp, webrtc and
the like), GLib's girepository and gthread, pcre2-posix, wayland-server and
wayland-cursor, libdconf (the GSettings module carries its own copy of what it
uses) and cairo's trace preload: 34 objects, about 5 MB.

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
the cache format version. From dconf only the GSettings module ships; the host
runs `dconf-service`.

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

## runtime-licenses.json

Every component, every Rust crate compiled in and the Rust standard library,
each with its licence and its licence files, for the package's copyright file.
Plain JSON, written by `write_licenses.py`:

```json
{
  "schema_version": 1,
  "archive": "runtime-licenses.tar.gz",
  "components": [
    {
      "name": "freetype",
      "version": "2.13.3",
      "license": "(FTL OR GPL-2.0-or-later) AND MIT AND MIT-open-group",
      "build_only": false,
      "license_files": [
        "components/freetype-2.13.3/LICENSE.TXT",
        "components/freetype-2.13.3/docs/FTL.TXT",
        "components/freetype-2.13.3/docs/GPLv2.TXT",
        "components/freetype-2.13.3/src/base/fthash.c.notice",
        "components/freetype-2.13.3/src/bdf/README.notice",
        "components/freetype-2.13.3/src/pcf/README.notice",
        "components/freetype-2.13.3/src/pcf/pcfutil.c.notice"
      ]
    }
  ],
  "crates": [
    {
      "component": "librsvg",
      "name": "cssparser",
      "version": "0.35.0",
      "license": "MPL-2.0",
      "source": "crates.io",
      "checksum": "<sha256 from librsvg's Cargo.lock>",
      "license_files": ["crates/cssparser-0.35.0/LICENSE"]
    },
    {
      "component": "librsvg",
      "name": "fxhash",
      "version": "0.2.1",
      "license": "Apache-2.0 OR MIT",
      "source": "crates.io",
      "checksum": "<sha256 from librsvg's Cargo.lock>",
      "license_files": ["standard/Apache-2.0.txt", "standard/MIT.txt"]
    }
  ],
  "rust_std": {
    "version": "1.97.1",
    "license": "(Apache-2.0 OR MIT) AND Unicode-3.0",
    "license_files": [
      "rust-std/COPYRIGHT-library.html",
      "rust-std/licenses/Apache-2.0.txt",
      "rust-std/licenses/MIT.txt",
      "rust-std/licenses/Unicode-3.0.txt"
    ]
  },
  "license_refs": {
    "LicenseRef-glib-gbsearcharray": "components/glib-2.88.3/glib/gbsearcharray.h.notice"
  }
}
```

| Field | What |
|---|---|
| `archive` | The tarball the paths are relative to, beside this file |
| `components` | Every component of the lock file, sorted by name |
| `components[].license` | The component's `license` in the lock file, an SPDX expression |
| `components[].build_only` | `true` for a component none of whose own files ship (wayland-protocols, whose XML GTK compiles into code) |
| `crates` | Every crate compiled into a Rust component, sorted by component, name and version, with librsvg's three workspace members among them |
| `crates[].component` | The lock file component the crate is compiled into |
| `crates[].license` | The SPDX expression the crate declares in its `Cargo.toml`, with Cargo's old `/` written as ` OR ` (`Apache-2.0/MIT` is recorded as `Apache-2.0 OR MIT`) |
| `crates[].source` | `crates.io` for a vendored crate, `path` for a member of the component's own workspace |
| `crates[].checksum` | The crate's sha256 in the component's `Cargo.lock`, which cargo checked; `null` for `path` |
| `rust_std` | The standard library, which every Rust object links; `null` when no component is Rust. `COPYRIGHT-library.html` carries every notice of the standard library and its dependencies, for every target |
| `license_files` | Paths inside `archive`, each a file to reproduce in the copyright file; never empty. A workspace member without its own licence file names its source tree's. The four crates above name `standard/` texts, which two of them share, and so do the components whose `standard_license_texts` names one (`standard/Unicode-3.0.txt`, `standard/LGPL-2.0-or-later.txt` and the others the Debian audit names). A component's `<path>.notice` holds the lines of a source file its `license_excerpts` names, the excerpts of one file in the lock's order (GLib's `gchecksum.c.notice` holds two) |
| `license_refs` | Each `LicenseRef-` any expression names, and the one path inside `archive` that holds its text; every one named is here, and every one here is named |

`runtime-licenses.tar.gz` holds exactly the files the index names:
`components/<name>-<version>/<path in its source tree>` (with `.notice` added
for an excerpt), `crates/<name>-<version>/<file>`, `standard/<SPDX id>.txt` and
`rust-std/<path>`. Integrity comes from `SHA256SUMS` and
`runtime-manifest.json`, which record both files. The texts come from the
pinned tarballs and texts, the vendored crates and the toolchain, and are written in the
export step, so `--verify` covers them.

`runtime-crates.tar.gz` is the source archive's: under
`<source_dir>/_crates/<name>-<version>/`, the whole source of every crates.io
crate in the index.

## Deviations from the amendment

| Amendment | What is true | Why |
|---|---|---|
| A§3.2 ships `lib/gtk-4.0/4.0.0/immodules/*.so` | GTK 4 has no loadable input modules | `GtkIMContextSimple` and the Wayland `text-input-v3` context are compiled into libgtk |
| A§4.1 lists neither expat nor pixman | Both are private | fontconfig needs an XML parser and cairo needs pixman |
| A§4.1 keeps libstdc++ off the host list | It is on it | webrtc-audio-processing is C++, and the plan adds it with a symbol ceiling |
| A§4.2 says `build_system` is `meson`, `autotools` or `cargo` | `meson`, `autotools` or `cmake` | libjpeg-turbo and abseil build with CMake; librsvg's meson build drives cargo itself |
| A§4.2 computes the cache key from the lock file, the patches and the Dockerfile | It also covers `build_runtime.sh`, `write_manifest.py`, `drop_unreachable.sh`, `write_crates.py` and `write_licenses.py` | The script sets the compiler, linker and archiver flags, the pruning decides what ships, and the manifest and licence index are published with the tree; a change to any of them under an unchanged key would publish a different runtime under the old key |
| A§4.2 builds the runtime in an image layer | The image is the toolchain; the compile is a container run | A failed component resumes, and the source cache is copied in rather than re-fetched |
| A§4.2 sets `SOURCE_DATE_EPOCH` from the lock file's commit | It is the lock file's `source_date_epoch` field | A commit date moves under a rebase; a field is covered by the cache key |
| A§4.3 puts the fontconfig cache under `$XDG_CACHE_HOME/fermix-desktop/fontconfig` | It shares `~/.cache/fontconfig` | The host `fonts.conf` names the cache directory and wins over a compiled-in default |

## Bumping a component

1. Change the version, url and sha256 in `RUNTIME.lock.json`. Fetch the tarball
   and read its digest; never copy one from a web page.
2. Read the requirements in the new version's `meson.build` and in the
   `meson.build` of everything that depends on it, and its licence notes: a
   new bundled directory, notice or Unicode table changes its expression (the
   tables above), and an excerpt whose lines moved fails the build until its
   range and sha256 are read again. Read Debian's copyright file for the
   package again, and hold each licence it names that the expression lacks to
   the new tarball, as the Debian audit above does.
3. `build_runtime_test.sh`. With the tarball in the source cache it also holds
   every meson flag to the component's own option file.
4. `build_runtime.sh --container`, then `smoke_runtime.sh`.

A GTK or libadwaita point release can change rendering, so the window's
reviewed captures are taken again after one.

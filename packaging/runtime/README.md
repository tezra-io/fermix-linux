# The private toolkit runtime

The `fermix-desktop` package carries its own GTK 4.16 and libadwaita 1.6, and
everything under them the host cannot be relied on to supply, in a private prefix
at `/usr/lib/fermix-desktop`. This directory is what builds that prefix. The
design it implements is
[`docs/design/SINGLE_PACKAGE_AMENDMENT.md`](../../docs/design/SINGLE_PACKAGE_AMENDMENT.md),
sections 3.2, 4 and 8.

```
RUNTIME.lock.json      every component: version, purl, url, sha256, build flags, licence
build_runtime.sh       fetch, build, wire, check and export it
build_runtime_test.sh  the gate's own refusals, and the lock file's rules
fetch_source.sh        one fetch, one digest rule, shared by the two above
package_sources.sh     the LGPL source archive published beside the packages
smoke_runtime.sh       run a libadwaita window on ubuntu:22.04 from the built tree
smoke/runtime_smoke.c  the twenty lines that window is
write_manifest.py      runtime-manifest.json: the lock plus every file's sha256
compare_manifest.py    what --verify compares with
check_options.py       holds every meson flag to the option upstream declares
patches/               per-component patches, named by the lock file. Normally empty
```

## Running it

```sh
packaging/runtime/build_runtime.sh --container   # build, about an hour
packaging/runtime/build_runtime.sh --container --fresh   # ... from an empty prefix
packaging/runtime/smoke_runtime.sh               # prove it runs on the oldest target
packaging/runtime/build_runtime.sh --verify      # rebuild and compare, another hour
packaging/runtime/build_runtime.sh --print-key   # the key of section 4.2, no daemon needed
packaging/runtime/build_runtime_test.sh          # the gate's refusals, seconds
packaging/runtime/package_sources.sh 0.10.5      # the LGPL source archive
```

`--container` resumes: it skips every component a previous run of this same lock
file already installed, so a failure late in the list does not rebuild the whole
toolkit. `--print-key` is the one mode that writes to standard output, because
its output is the key; a release workflow uses it to decide whether to pull
`ghcr.io/tezra-io/fermix-desktop-runtime:<key>-<arch>` or build it.

`package_sources.sh <version>` writes
`fermix_desktop_runtime_sources_<version>.tar.gz` — every locked tarball, every
patch, the lock file, the Dockerfile and the build script, so the written offer
of amendment section 4.6 is a file rather than a promise. It takes each tarball
from the download cache when it is there and from the lock file's URL when it is
not, checking every one against the recorded sha256 either way, because a source
archive whose contents are not what was built is worse than none: it looks like
compliance. Fetching is the default rather than a flag, because a release that
could only produce this archive from a build cache would fail the first time
that cache was evicted — the steady state of a lock file that deliberately does
not change for months — on the one asset that discharges a licence obligation.
`--no-fetch` refuses the network, for hermetic checks and for the gate.

Inside, it keeps this repository's layout — `sources/` beside
`packaging/runtime/` and `packaging/docker/` — because `build_runtime.sh` finds
its lock file, its patches, its Dockerfile and the library it sources by paths
relative to itself. A flattened archive ships a build script that cannot run,
which is exactly what the first version of this did; unpacking it and running
the documented command is what caught it. From the top of the archive:

```sh
FERMIX_RUNTIME_SOURCES="$PWD/sources" packaging/runtime/build_runtime.sh --container
```

The archive is deterministic: sorted entries, zeroed timestamps, numeric owners,
`gzip -n`. Two runs from the populated cache and one from an empty directory
that fetched all 29 tarballs produced identical bytes — sha256
`c639129f75337ce4cba4a9f343fb50fb683d9789f70b8e864d624af8473b6757`, 129 MB, 36
files. The archive is a function of the lock file and nothing else.

The output lands in `packaging/out/runtime/`, which is already ignored:

| File | What |
|---|---|
| `runtime-<arch>.tar` | The shipped tree, rooted at `usr/lib/fermix-desktop` |
| `runtime-dev-<arch>.tar` | The same tree with headers, `.pc` files and the build-time binaries |
| `runtime-manifest.json` | The lock file plus the sha256 of every shipped file |
| `cache-key` | The key of amendment section 4.2, from the lock, the patches and the Dockerfile |

Environment variables move things. `FERMIX_RUNTIME_OUT` moves the output.
`FERMIX_RUNTIME_SOURCES` moves the tarball cache, which defaults to
`~/.cache/fermix-desktop-runtime/sources` so a second run does not re-download.
`FERMIX_RUNTIME_JOBS` caps compile parallelism and defaults to 4.
`FERMIX_RUNTIME_PREFIX_VOLUME` and `FERMIX_RUNTIME_BUILD_VOLUME` name the two
docker volumes the build works in. `DOCKER_HOST` chooses the daemon, as it does
for any docker command, because this script never names one.

## What slice 4 and slice 5 need from this

**Slice 4, the package.** Take `runtime-<arch>.tar` and unpack it at the root of
the staged tree: its contents are already `usr/lib/fermix-desktop/...` with the
final modes, so `tar -xf runtime-<arch>.tar -C <stage>` is the whole step. The
tree is already pruned of headers, `.pc` files, static libraries, documentation,
introspection data and every build-time binary. There is no `bin/` at all —
not even an empty one — so the directory the application ELF installs into at
`/usr/lib/fermix-desktop/bin/fermix-desktop` is the package's to create.
`libexec/` does survive, for `gio-launch-desktop`, which GLib spawns by its
compiled-in path. Nothing in the tree needs a post-install step:
`loaders.cache`, `gschemas.compiled` and the icon theme cache are generated at
build time against the final absolute prefix. `runtime-manifest.json` is what
ships at `/usr/share/doc/fermix-desktop/runtime-manifest.json`, and its `lock`
object carries a `license` field per component, which is what
`packaging/copyright` is generated from.

**Slice 5, the application.** Build against the dev variant. Unpack
`runtime-dev-<arch>.tar` at `/` in the build image and set:

```sh
export PKG_CONFIG_PATH=/usr/lib/fermix-desktop/lib/pkgconfig:/usr/lib/fermix-desktop/share/pkgconfig
export PATH=/usr/lib/fermix-desktop/bin:$PATH   # glib-compile-resources, for build.rs
```

Both `pkgconfig` directories, because exactly one `.pc` file — `wayland-protocols.pc`
— installs under `share/pkgconfig` and the other 63 under `lib/pkgconfig`.

Link the application ELF with `-Wl,-rpath,'$ORIGIN/../lib'
-Wl,--enable-new-dtags` and nothing else; it lives at
`/usr/lib/fermix-desktop/bin/fermix-desktop`, so `$ORIGIN/../lib` is the private
`lib/`. `packaging/runtime/smoke_runtime.sh` compiles and links a C program
exactly that way, so it is a working example of the link line.

Three things the application must **not** do, because they are already compiled
in and an environment variable would only be a way to get them wrong:
`GIO_MODULE_DIR`, `GDK_PIXBUF_MODULE_FILE` and `XDG_DATA_DIRS`. Only
`GSETTINGS_SCHEMA_DIR` needs setting, to
`/usr/lib/fermix-desktop/share/glib-2.0/schemas`, and it prepends rather than
replaces, so the host's `org.gnome.desktop.interface` is still found.

**One thing the application must do**, and it is the single place where "the
paths are compiled in" does not hold:

```c
gtk_icon_theme_add_search_path(theme, "/usr/lib/fermix-desktop/share/icons");
```

GTK builds its icon search path from `XDG_DATA_DIRS` and the user's data
directory, not from the prefix it was compiled with, so the bundled Adwaita
theme is invisible until something names it. Setting `XDG_DATA_DIRS` instead
would be the wrong fix, because it moves every other data lookup with it. This
is not a precaution: the smoke asserted the path was there for free, and failed,
which is how it was found. The theme and its cache are in the shipped tree
either way, so the symptom is a window that opens with no icons and looks like a
theming bug rather than a packaging one.

## Decisions, and why

**The prefix is the final path, inside the container.** Every component is
configured with `--prefix=/usr/lib/fermix-desktop` and installed there. A GLib
configured with that prefix compiles it in as its GIO module directory, and a
gdk-pixbuf configured with it compiles in the private `loaders.cache` path, so
both are right on a user's machine with no environment variable at all. Nothing
is built under a staging prefix and relocated.

Inside the container the prefix and the build tree are docker volumes, mounted at
exactly those paths. That is not a relocation: the compiler, every configure
script and every generated cache see `/usr/lib/fermix-desktop`, and the tree
leaves as the exported tarballs. They are volumes and not bind mounts for a
reason worth writing down, because it costs an hour to rediscover: where the
Docker daemon runs in a virtual machine, a bind-mounted host directory carries
the **host's** clock while the container carries the machine's, the two differ by
about a second, and ninja stops on every file it finds in its own future with
"Clock skew detected". A volume is written by the daemon itself, so there is one
clock. They are not the container's writable layer either, because `--rm` would
discard a half-finished build with the container.

**A run resumes.** `build_all` writes a stamp per installed component into the
build volume and skips any component whose stamp is already there, so a failure
in the twenty-eighth component costs the twenty-eighth component and not the
first twenty-seven. The stamp is written *after* install, so a component that
died half way through is built again from the top. What makes this safe rather
than a source of stale trees is that the volumes carry a label naming the cache
key they were filled under: the host side discards both whenever that key moves,
which is whenever the lock file, a patch or the Dockerfile changes. `--fresh`
discards them on demand, and `--verify` always uses its own empty pair, since a
verification that resumed from the first build's volumes would be comparing a
tree with itself.

**The daemon is whichever one the environment names.** The script never passes
`-H` and never names a socket, so `DOCKER_HOST` and `docker context` both decide
it, and the same command runs against a desktop VM or a system daemon:

```sh
DOCKER_HOST=unix:///var/run/docker.sock FERMIX_RUNTIME_JOBS=12 \
  packaging/runtime/build_runtime.sh --container
```

The container installs into `/usr/lib`, so it runs as root. With a native daemon
the output directory is a host path, which would leave root-owned tarballs in the
invoking user's working tree; the caller passes its uid and gid in and the build
hands the output directory back before it exits.

**RUNPATH is written once, by patchelf, after install.** Every build system has
its own idea of what to strip out of an rpath at install time, and arguing with
five of them is five chances to be wrong. `fix_runpaths` computes the correct
relative path from where each object actually sits — `$ORIGIN` for a library in
`lib/`, `$ORIGIN/../lib` for a binary in `bin/`, `$ORIGIN/../../../../lib` for a
pixbuf loader four directories down — writes it with `patchelf`, and
`check_runpaths` then reads every one back and refuses anything else, including
an `RPATH` where a `RUNPATH` was asked for.

**The host boundary is a check, not a comment.** `check_needed_entries` reads the
`NEEDED` entries of every private object and refuses any that is neither a file
inside the prefix nor on the `host_libraries` list in the lock file. That list is
section 4.1's host column, written down. Slice 4 owns the same check at the
package level; this one exists so the runtime cannot leave this directory wrong.

**What the check cannot do**, stated plainly because it took another slice to
find the hole: it proves nothing UNDECLARED is linked, and it can never prove
the declared list is COMPLETE. A `dlopen`ed library appears in no `NEEDED`
entry, so it is invisible to this check — and to anyone, like me, who verifies
the list against `NEEDED` entries. That is not hypothetical. `libepoxy` names
six GL libraries it may load at runtime, of which only three were declared until
slice 4's package aborted under Xvfb on a missing `libGLESv2.so.2`. My smoke
could not have caught it either: it sets `GSK_RENDERER=cairo` and never takes
the GL path. All six are on the list now, found by sweeping every shipped ELF
for embedded `lib*.so.N` strings that are neither private files nor already
declared:

```sh
strings -a lib/libepoxy.so.0 | grep -E 'lib[A-Za-z0-9_+-]+\.so\.[0-9]+'
# libGL.so.1 libEGL.so.1 libGLESv2.so.2 libGLESv1_CM.so.1 libGLX.so.1 libOpenGL.so.0
```

One of those six names nothing that exists. libepoxy asks for **`libGLX.so.1`**,
and glvnd ships `libGLX.so.0` — absent on Ubuntu 22.04 and Debian 13, provided
by nothing on AlmaLinux 9 and Fedora 44, and confirmed here by installing
`libglx0` in a container and finding only `libGLX.so.0`. That `dlopen` therefore
fails on every target we support, and epoxy falls back to `libGL.so.1`, which is
declared and present. `host_libraries` records `libGLX.so.0`: the list is a
boundary describing what may be loaded from a host, and a row naming a file no
distribution ships would describe nothing. Slice 4 found this against the real
package indexes of all four targets, which is the only way it could have been
found.

Completeness is reachable only by running the thing on a bare machine. The smoke
does that for the cairo path and slice 4's install test does it for the GL path;
neither replaces the other.

**The renderer is why the gap survived**, and that is worth naming because it
was not bad luck. `smoke_runtime.sh` sets `GSK_RENDERER=cairo`, so it never
reaches libepoxy's `dlopen` at all: every GL library could have been missing and
it would still have passed. Slice 4's install test found the hole only because
Xvfb has no GPU and GTK fell through to GLES. Neither check was looking for it;
the difference was which renderer ran. A smoke that also ran once with the GL
renderer against a software GL stack closes that deliberately, and now does:
`smoke_runtime.sh` runs twice, once with `GSK_RENDERER=cairo` and once with
`gl`, the GL pass installing Mesa's software rasteriser into the **smoke image**
— not into the package's relations, since a user's machine has a real driver.
The program refuses to pass if GTK fell back to cairo, because a silent fallback
is right for a user and useless for a test: it would turn the point of the run
into a green tick. It caught its own bug on the first run, having looked for a
renderer named `Gl` when GTK 4.16 spells it `GskGLRenderer`.

The sweep is also how "is that all of them?" gets an answer. Run over every ELF
in the shipped tree, reporting names that appear as strings but not among that
object's own `NEEDED` entries, it finds exactly six — all in `libepoxy`, and
nothing at all in GTK, gdk-pixbuf, pango or the GIO modules. In particular
`libGLdispatch.so.0`, `libgbm.so.1` and `libdrm.so.2` are named **nowhere** in
this tree. They are on `host_libraries` because §4.1 lists them and a host GL
stack brings them, which is a statement about the host's packaging rather than
about anything this runtime loads — so the package is right not to declare them
as its own relations. I had written that they "arrive behind libGL"; that was
reasoning, and this measurement is what replaced it.

The list is a floor and not a census, and the difference matters when anyone is
tempted to trim it to what the tree currently links. Measured on the real
shipped tree, only 20 of the declared SONAMEs appear in any `NEEDED` entry. Some
of the rest are genuinely unused today (`libzstd`, `liblzma`, `libdbus-1`,
`libX11-xcb`); four are glibc 2.34 stubs whose symbols moved into `libc.so.6`
(`libdl`, `libpthread`, `librt`, `libresolv`); one is the other architecture's
loader; and the GL family above is dlopened and **must never be pruned**. A
package that drops those rows because no `NEEDED` entry names them installs
cleanly and fails when GTK creates its first GL context.

**Vulkan is disabled.** GTK 4.16 can build a Vulkan renderer, and doing so links
`libvulkan.so.1`, which is not on section 4.1's host list and would be a
twenty-third undeclared host dependency. GTK's default renderer in 4.16 is the
GL one, which reaches the driver through libepoxy's `dlopen` of the host
`libGL.so.1` and `libEGL.so.1` and needs nothing declared beyond what already is.
Declaring Vulkan would mean adding a host row, a package relation in both
families and a smoke row, to gain a renderer the application does not ask for.

**GTK's autohinting does not use HarfBuzz.** FreeType is built with
`-Dharfbuzz=disabled`, which breaks what would otherwise be a dependency cycle
between the two. HarfBuzz is still what shapes every string the window draws;
what FreeType loses is the auto-hinter's ability to group glyphs by script, which
matters for hinting quality on fonts with no hinting of their own, at small
sizes. A two-pass build (FreeType, HarfBuzz, FreeType again) would recover it
and would mean one component appearing twice in the build order for a difference
nobody has reported seeing. If that changes, the second pass is four lines.

**libstdc++ is not a host dependency.** HarfBuzz is the only C++ component, and
it is built with `-Dwith_libstdcxx=false`, which is upstream's own option for
exactly this: it compiles without exceptions or RTTI and links no C++ runtime.
`libstdc++.so.6` is therefore absent from every `NEEDED` entry, and
`build_runtime_test.sh` refuses to let it onto the host list.

libtiff was the exception, and the host-boundary check is how it was found
rather than a reviewer. libtiff builds `libtiffxx`, a C++ wrapper around the
same library, and that one does link `libstdc++`. Nothing in this runtime uses
it — gdk-pixbuf's TIFF loader links `libtiff` — so it is not built at all:
`--disable-cxx`. Adding `libstdc++.so.6` to the host list instead would have
been a whole C++ runtime declared as a dependency of a desktop application for
the sake of a wrapper no code calls.

**The private fontconfig reads the host's `/etc/fonts`.** It is configured with
`-Dbaseconfig-dir=/etc/fonts -Dconfig-dir=/etc/fonts/conf.d`, so it finds the
host's configuration, the host's aliases and the host's installed fonts. The
prefix's own `etc/` is pruned out of the shipped tree, so there is no second
configuration to be confused about. A newer fontconfig parsing an older
configuration is the direction that works.

The cache is a known departure from amendment section 4.3, which says the private
cache lives under `$XDG_CACHE_HOME/fermix-desktop/fontconfig`. The host
`fonts.conf` names `<cachedir prefix="xdg">fontconfig</cachedir>`, and a
compiled-in default loses to the configuration file, so the private fontconfig
writes to `~/.cache/fontconfig` beside the host's. That is safe rather than
merely tolerable: a fontconfig cache file's name carries its format version, so a
newer fontconfig writes files the host's copy ignores and reads none of the
host's. Isolating it would mean the application setting `FONTCONFIG_FILE` or
`XDG_CACHE_HOME`, and both are worse than sharing a directory of files that never
collide.

**librsvg's crates come from crates.io at build time.** The librsvg tarball is
pinned by sha256 and carries its own `Cargo.lock`, and cargo verifies every crate
it fetches against the checksums in that lock, so the dependency set is pinned
transitively by the one digest in `RUNTIME.lock.json`. The build container
therefore needs the network, which it needs for the tarballs anyway.

**Reproducibility is claimed only as far as it is checked.** `SOURCE_DATE_EPOCH`
comes from the lock file, the compiler is pinned by the base image digest, the
prefix is a fixed absolute path, and `-ffile-prefix-map` and
`--remap-path-prefix` keep the build directory out of the output.
`build_runtime.sh --verify` rebuilds and compares `runtime-manifest.json` file by
file, and `check_no_build_paths` refuses a tree with a build path in it. What
`--verify` reports is the truth about this toolchain; it is not an assertion that
every upstream component is bit-reproducible.

**The published digests are covered too.** `runtime-manifest.json` carries an
`archives` object with the sha256 of each exported tarball, and `--verify` fails
if either moves. The tree and the tarball are two different claims: two archives
of an identical tree can differ through entry order or metadata, and it is the
archive's digest that gets published, so a rebuild reproducing the tree but not
the tar would publish a digest that verifies for nobody. `write_tar` passes
`--sort=name` under `LC_ALL=C`, making the entry order a property of the tree
rather than of the filesystem. Note that `--sort=name` is a directory walk with
each directory's entries sorted, which is deliberately **not** the sequence a
flat `LC_ALL=C sort` of every path produces — `/` sorts before most characters,
so a flat sort interleaves a directory's files with its subdirectories'. Both
are deterministic; only one is what tar writes, and mistaking the difference for
readdir order cost a bug report. Two consecutive exports of the same tree
produced identical tarball digests.

**The claim includes the job count.** `FERMIX_RUNTIME_JOBS` must not change the
output, because CI runners and developer machines have different core counts and
a tree that depends on either is not reproducible in any useful sense. One thing
did depend on it: annobin writes a `.gnu.build.attributes` section whose size
varies with how a compile was parallelised, and it made librsvg's pixbuf loader
compare as different between a 12-job and an 8-job build — identical GNU build
ID, identical strings, every allocated byte identical, 36 bytes of annotation
apart. `strip_objects` now removes those sections from every private ELF, so the
shipped tree carries no section whose contents depend on the machine that built
it. `.comment` is left alone: it names the compiler and the linker, and it was
byte-identical across every build compared here.

## Deviations from the amendment

Each of these is a place where the amendment's text and what upstream actually
does disagree, and the code follows upstream.

| Amendment | What is true | Why |
|---|---|---|
| §3.2 ships `lib/gtk-4.0/4.0.0/immodules/*.so`, and §4.3 says GTK is built with "its own `ibus` input module" | **GTK 4 has no loadable input modules and no ibus module.** `GtkIMContextSimple` and the Wayland `text-input-v3` context are compiled into `libgtk-4`, and `gtk-4.16.7/modules/` contains only `media` and `printbackends`, both disabled here | Nothing to build and nothing to ship. §4.3's conclusion is unchanged and now unconditional: on Wayland the compositor handles input methods, and on X11 an fcitx5 user falls back to XIM. Slice 4 must drop that row from `INSTALLED_PATHS` |
| §4.1 lists neither **expat** nor **pixman** | Both are private components | fontconfig requires an XML parser and cairo requires pixman. Neither is on the host list, so neither may be linked from the host |
| §4.2 says `build_system` is `meson`, `autotools` or `cargo` | It is `meson`, `autotools` or `cmake` | libjpeg-turbo 3.x builds with CMake only. librsvg is a meson project that invokes cargo itself, so no component has `cargo` as its own build system |
| §4.2 says `Dockerfile.runtime` "builds the private runtime" as an image whose content is the prefix | `Dockerfile.runtime` builds the **toolchain**; the compile is a `docker run` and the publishable image is made by `docker import` of `runtime-<arch>.tar` | A `docker build` layer cannot see a source cache, so every iteration would re-download every tarball. The published image is byte-identical either way: its entire content is `usr/lib/fermix-desktop` |
| §4.2 says `SOURCE_DATE_EPOCH` is "set from the lock file's own commit" | It is the `source_date_epoch` field **in** the lock file | A commit date changes under a rebase, which would make the same lock file produce two different trees. A field in the file is covered by the cache key |
| §4.3 sets `GDK_PIXBUF_MODULE_FILE`, `GIO_MODULE_DIR` and `XDG_DATA_DIRS` | Only `GSETTINGS_SCHEMA_DIR` is set | All three are compiled in at the final prefix, which is what building at that prefix buys. The icon theme is the **exception**, and I had this wrong until the smoke failed on it: GTK builds its icon search path from `XDG_DATA_DIRS` and the user's data directory, not from its compiled-in prefix, so the bundled Adwaita theme is invisible until the application calls `gtk_icon_theme_add_search_path("/usr/lib/fermix-desktop/share/icons")`. That call appends, so the host's theme is still searched first, which is the host-first ordering §4.3 asks for — reached by an explicit call rather than by a compiled-in default |
| §4.3 puts the private fontconfig cache under `$XDG_CACHE_HOME/fermix-desktop/fontconfig` | It shares `~/.cache/fontconfig` with the host | Above. The host `fonts.conf` wins over any compiled-in default, and cache file names carry their format version so the two never collide |
| §4.1 does not say whether `libX11-xcb`, `libxcb-render` or `libxcb-shm` are host | They are, and they are on the list | Cairo's XCB surface links them. They are the same frozen wire-protocol clients as the rest of the X11 row. The smoke found this the expensive way: its first run on a stock ubuntu:22.04 died on `libxcb-render.so.0`, which was correctly on the host list and missing from the packages the smoke installed |
| §4.1 does not mention **libtiffxx** | libtiff is built `--disable-cxx` | Its C++ wrapper links `libstdc++.so.6`, which is not a host library, and nothing in the runtime calls it — gdk-pixbuf's TIFF loader uses the C API. The host-boundary check refused the tree until this was fixed |

## Bumping a component

1. Change the version, the URL and the sha256 in `RUNTIME.lock.json`. The digest
   is not optional and is not copied from a web page: fetch the tarball and read
   it.
2. `packaging/runtime/build_runtime_test.sh` — seconds, and it catches a lock
   file that no longer holds. Once the tarball is in the source cache it also
   reads the component's own `meson.options` out of it and refuses a flag
   upstream does not declare, or one it has deprecated. That check exists
   because the alternative way to learn that `-Dlzma=false` should have been
   `-Dlzma=disabled` is an hour of compiling followed by one line from meson.
3. `packaging/runtime/build_runtime.sh --container` — an hour.
4. `packaging/runtime/smoke_runtime.sh`.
5. The cache key changes, so `.github/workflows/runtime.yml` publishes a new
   image on merge and every later build pulls it.

A GTK or libadwaita point release also needs the reviewed captures re-taken,
because a point release can change rendering; that is
`scripts/capture.sh`, and it is slice 5's.

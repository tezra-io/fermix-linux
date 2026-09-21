# Patches

One file per patch, named `*.patch`, applied with `patch -p1` inside the unpacked
source, and named by the `patches` list of the component's entry in
`../RUNTIME.lock.json`. The build
refuses a lock file that names a patch this directory does not hold.

The directory is normally empty, and that is the point: a patch here is a
divergence from an upstream release that every later bump has to carry forward,
so each one needs a comment at the top saying what it fixes, where it was sent
upstream, and what would let it be dropped. `patches/` is part of the runtime
cache key, so adding, changing or removing one rebuilds the runtime; this file
does not.

Every patch also ships in the release's source tarball,
`fermix_desktop_runtime_sources_<version>.tar.gz`, which is how the LGPL
obligation of amendment section 4.6 is met.

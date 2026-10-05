# Patches

One file per patch, named `*.patch`, applied with `patch -p1` inside the unpacked
source and named by the `patches` list of its component in
`../RUNTIME.lock.json`. The build refuses a lock file that names a patch this
directory does not hold.

A patch is a divergence every later bump has to carry, so each one starts with
what it fixes, whether it went upstream, and what would let it be dropped. The
patches are part of the runtime's cache key, so adding, changing or removing one
rebuilds the runtime; this file is not.

Every patch also ships in the release's source archive,
`fermix_desktop_runtime_sources_<version>.tar.gz`, which is how the LGPL source
obligation of amendment section 4.6 is met.

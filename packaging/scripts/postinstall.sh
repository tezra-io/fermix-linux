#!/bin/sh
# Refresh the two caches a desktop reads, after this package's files land
# (M38 section 5.3). Runs as root, from dpkg and from rpm alike.
#
# This is the desktop half of the package's postinstall. The script that ships
# is assembled at build time by scripts/assemble_maintainer.sh: the engine's own
# fragment first, byte for byte out of the engine artifact, then this one
# (amendment section 3.3). Each half runs in a subshell, so the `exit 0` below
# ends this half rather than the script.
#
# AppStream composition needs the desktop entry, the metainfo and the icon
# together; missing any one of them leaves the application correctly installed
# and invisible in GNOME Software and KDE Discover. These two commands are what
# make the freshly installed three visible without a logout.
#
# Both tools are guarded rather than depended on: a minimal host may carry
# neither, and an icon cache that was not rebuilt is a stale icon, not a broken
# install. Nothing here operates any user's service manager, and nothing here
# executes anything Fermix ships.
set -eu

if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database -q /usr/share/applications || true
fi

if command -v gtk-update-icon-cache >/dev/null 2>&1; then
  gtk-update-icon-cache -f -q /usr/share/icons/hicolor || true
fi

exit 0

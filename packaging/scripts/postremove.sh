#!/bin/sh
# Refresh the same two caches after this package's files are taken away
# (M38 section 5.3). Runs as root, from dpkg and from rpm alike.
#
# A launcher entry that survives its binary is worse than no entry: it opens
# nothing and says nothing. Rebuilding both caches on removal is what stops that.
#
# This script removes no user data. The application keeps its window geometry
# and its selected pane under $XDG_STATE_HOME/fermix-desktop and its autostart
# entry under $XDG_CONFIG_HOME/autostart, both of which belong to the person
# rather than to the package, and neither of which is package-owned to begin
# with. It operates no user's service manager.
set -eu

if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database -q /usr/share/applications || true
fi

if command -v gtk-update-icon-cache >/dev/null 2>&1; then
  gtk-update-icon-cache -f -q /usr/share/icons/hicolor || true
fi

exit 0

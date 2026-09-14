#!/usr/bin/env bash
#
# One string, six places (M38 section 5.3).
#
# The application identity is `io.tezra.Fermix`. Nothing in the platform
# enforces that the six places agree, and every failure mode is a silent
# misassociation rather than an error: the application missing from the
# launcher, a second launch opening a second window instead of raising the
# first, a generic icon in the dash, a window that does not group under its
# launcher, invisibility in the software centres, a portal connection whose
# identity nobody can establish. So a script asserts it.
#
# The six, and where each one lives:
#
#   1. the desktop entry's basename, and its StartupWMClass, which is the X11
#      half of the Wayland app_id the toolkit takes from the application id
#   2. the D-Bus well-known name, and the object path the application and its
#      resource bundle are registered under
#   3. the application id the toolkit is built with
#   4. the AppStream id
#   5. the icon file names under hicolor
#   6. the activation unit's BusName, which is where a launch from a cold
#      session gets its identity from
#
# And one relation between two of them: the D-Bus service must name the unit, or
# activation from a cold session never runs through the user manager and the
# process carries no unit name for a portal to derive an identity from.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP_ID="io.tezra.Fermix"

failures=0

fail() {
  echo "check_app_identity: $*" >&2
  failures=$((failures + 1))
}

require_in_file() {
  local what="$1" path="$2" needle="$3"
  if [ ! -f "$ROOT_DIR/$path" ]; then
    fail "$what: $path is missing"
    return
  fi
  if ! grep -qF -- "$needle" "$ROOT_DIR/$path"; then
    fail "$what: $path does not carry $needle"
    return
  fi
  echo "  ok: $what ($path)"
}

require_file() {
  local what="$1" path="$2"
  if [ ! -f "$ROOT_DIR/$path" ]; then
    fail "$what: $path is missing"
    return
  fi
  echo "  ok: $what ($path)"
}

# ---- 3. the application ----------------------------------------------------

require_in_file "application id" \
  "App/Fermix/src/app.rs" "pub const APPLICATION_ID: &str = \"$APP_ID\";"

require_in_file "resource prefix" \
  "App/Fermix/src/app.rs" "pub const RESOURCE_PREFIX: &str = \"/io/tezra/Fermix\";"

# X11 takes a window's WM_CLASS from the program name, and the launcher entry
# matches StartupWMClass against it, so the program name is the application id.
# Without this line a window on X11 announces the binary's own name, draws a
# generic icon in the dash and groups under nothing, and nothing reports it.
require_in_file "window class on X11" \
  "App/Fermix/src/app.rs" "glib::set_prgname(Some(APPLICATION_ID));"

require_in_file "resource bundle prefix" \
  "App/Fermix/resources/fermix.gresource.xml" "prefix=\"/io/tezra/Fermix\""

require_in_file "icon name in the bundle" \
  "App/Fermix/resources/fermix.gresource.xml" "$APP_ID.svg"

require_in_file "autostart entry basename" \
  "App/Fermix/src/session/autostart.rs" "pub const ENTRY_NAME: &str = \"$APP_ID.desktop\";"

# ---- 1. the launcher -------------------------------------------------------

require_file "desktop entry basename" "packaging/$APP_ID.desktop"
require_in_file "window class" "packaging/$APP_ID.desktop" "StartupWMClass=$APP_ID"
require_in_file "icon name in the launcher" "packaging/$APP_ID.desktop" "Icon=$APP_ID"

# ---- 2. the bus ------------------------------------------------------------

require_file "D-Bus service basename" "packaging/dbus/$APP_ID.service"
require_in_file "D-Bus well-known name" "packaging/dbus/$APP_ID.service" "Name=$APP_ID"

# ---- 4. the software centres -----------------------------------------------

require_file "metainfo basename" "packaging/$APP_ID.metainfo.xml"
require_in_file "AppStream id" "packaging/$APP_ID.metainfo.xml" "<id>$APP_ID</id>"
require_in_file "launchable" \
  "packaging/$APP_ID.metainfo.xml" "<launchable type=\"desktop-id\">$APP_ID.desktop</launchable>"

# ---- 5. the icon theme -----------------------------------------------------

for size in 16 22 24 32 48 64 128 256; do
  require_file "icon ${size}x${size}" \
    "packaging/icons/hicolor/${size}x${size}/apps/$APP_ID.png"
done
require_file "scalable icon" "packaging/icons/hicolor/scalable/apps/$APP_ID.svg"
require_file "symbolic icon" "packaging/icons/hicolor/symbolic/apps/$APP_ID-symbolic.svg"

# ---- 6. the activation unit ------------------------------------------------

require_file "activation unit basename" "packaging/systemd/app-$APP_ID.service"
require_in_file "activation unit bus name" \
  "packaging/systemd/app-$APP_ID.service" "BusName=$APP_ID"

# ---- and the relation between two of them ----------------------------------

require_in_file "the D-Bus service names the unit" \
  "packaging/dbus/$APP_ID.service" "SystemdService=app-$APP_ID.service"

# ---- nothing carries a second identity -------------------------------------

# The application and its packaging only: a gate's own test harness writes
# wrong identities on purpose, to prove this check fires.
# Sources and packaging only, never build output. `target` is gigabytes and a
# gate that reads it takes minutes to say what it says in a tenth of a second;
# `packaging/out` holds the built binary, whose string table puts the identity
# next to whatever string follows it and would read as a second identity every
# time. Both are ignored by git, and neither is a place an identity is authored.
strays="$(grep -rlE --exclude-dir=target --exclude-dir=out 'io\.tezra\.(FermixPet|Fermix[A-Za-z]+)' \
  "$ROOT_DIR/App" "$ROOT_DIR/packaging" 2>/dev/null || true)"
if [ -n "$strays" ]; then
  fail "a second identity appears in: $strays"
fi

if [ "$failures" -gt 0 ]; then
  echo "check_app_identity: $failures place(s) disagree about the application identity" >&2
  exit 1
fi

echo "check_app_identity: every place carries $APP_ID"

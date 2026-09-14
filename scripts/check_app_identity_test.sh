#!/usr/bin/env bash
#
# Exercise check_app_identity.sh's refusals against a throwaway copy of the
# repository. Every failure mode this gate exists for is silent at runtime, so
# the gate itself has to be seen failing.
#
# The copy carries all six places, because the gate now checks all six: the
# sources it started with, and the packaging files the release slice added.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP_ID="io.tezra.Fermix"

WORK="$(mktemp -d "${TMPDIR:-/tmp}/check-identity-test.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

fail() {
  echo "check_app_identity_test: $*" >&2
  exit 1
}

copy_repo() {
  local into="$WORK/$1"
  mkdir -p "$into/App/Fermix/src/session" "$into/App/Fermix/resources" \
    "$into/packaging/dbus" "$into/packaging/systemd" "$into/scripts"
  cp "$ROOT_DIR/scripts/check_app_identity.sh" "$into/scripts/"
  cp "$ROOT_DIR/App/Fermix/src/app.rs" "$into/App/Fermix/src/"
  cp "$ROOT_DIR/App/Fermix/src/session/autostart.rs" "$into/App/Fermix/src/session/"
  cp "$ROOT_DIR/App/Fermix/resources/fermix.gresource.xml" "$into/App/Fermix/resources/"
  cp "$ROOT_DIR/packaging/$APP_ID.desktop" "$into/packaging/"
  cp "$ROOT_DIR/packaging/$APP_ID.metainfo.xml" "$into/packaging/"
  cp "$ROOT_DIR/packaging/dbus/$APP_ID.service" "$into/packaging/dbus/"
  cp "$ROOT_DIR/packaging/systemd/app-$APP_ID.service" "$into/packaging/systemd/"
  cp -R "$ROOT_DIR/packaging/icons" "$into/packaging/"
  echo "$into"
}

expect_refusal() {
  local what="$1" repo="$2"
  if bash "$repo/scripts/check_app_identity.sh" >/dev/null 2>&1; then
    fail "$what was accepted"
  fi
  echo "  refused: $what"
}

echo "check_app_identity_test: the repository as it stands"
"$ROOT_DIR/scripts/check_app_identity.sh" >/dev/null || fail "the real repository does not pass"
echo "  accepted: the repository"

echo "check_app_identity_test: the copy it drives"
repo="$(copy_repo baseline)"
bash "$repo/scripts/check_app_identity.sh" >/dev/null ||
  fail "the copy the refusals are driven against does not pass on its own"
echo "  accepted: the copy, unbroken"

echo "check_app_identity_test: refusals in the application"

repo="$(copy_repo drifted-source)"
sed -i.bak 's|io\.tezra\.Fermix"|io.tezra.FermixApp"|' "$repo/App/Fermix/src/app.rs"
expect_refusal "the application id drifting in the source" "$repo"

repo="$(copy_repo drifted-prefix)"
sed -i.bak 's|/io/tezra/Fermix|/io/tezra/FermixApp|g' \
  "$repo/App/Fermix/resources/fermix.gresource.xml"
expect_refusal "the resource prefix drifting in the bundle" "$repo"

repo="$(copy_repo drifted-autostart)"
sed -i.bak 's|io\.tezra\.Fermix\.desktop|io.tezra.FermixPet.desktop|' \
  "$repo/App/Fermix/src/session/autostart.rs"
expect_refusal "the autostart entry naming another identity" "$repo"

repo="$(copy_repo drifted-prgname)"
sed -i.bak 's|glib::set_prgname(Some(APPLICATION_ID));|glib::set_prgname(Some("fermix-desktop"));|' \
  "$repo/App/Fermix/src/app.rs"
expect_refusal "a program name that is not the application id, which is the X11 window class" "$repo"

repo="$(copy_repo missing-source)"
rm "$repo/App/Fermix/src/app.rs"
expect_refusal "a place that is missing entirely" "$repo"

echo "check_app_identity_test: refusals in the packaging"

repo="$(copy_repo drifted-wmclass)"
sed -i.bak "s|StartupWMClass=$APP_ID|StartupWMClass=fermix-desktop|" \
  "$repo/packaging/$APP_ID.desktop"
expect_refusal "a window class that is not the application id" "$repo"

repo="$(copy_repo drifted-icon-name)"
sed -i.bak "s|Icon=$APP_ID|Icon=fermix|" "$repo/packaging/$APP_ID.desktop"
expect_refusal "a launcher icon named something the icon theme does not carry" "$repo"

repo="$(copy_repo drifted-bus-name)"
sed -i.bak "s|Name=$APP_ID|Name=io.tezra.FermixDesktop|" \
  "$repo/packaging/dbus/$APP_ID.service"
expect_refusal "a D-Bus name that is not the application id" "$repo"

repo="$(copy_repo unlinked-unit)"
sed -i.bak "s|SystemdService=app-$APP_ID.service|SystemdService=fermix-desktop.service|" \
  "$repo/packaging/dbus/$APP_ID.service"
expect_refusal "a D-Bus service that names a unit this package does not install" "$repo"

repo="$(copy_repo drifted-appstream)"
sed -i.bak "s|<id>$APP_ID</id>|<id>ai.fermix.Desktop</id>|" \
  "$repo/packaging/$APP_ID.metainfo.xml"
expect_refusal "an AppStream id that is not the application id" "$repo"

repo="$(copy_repo drifted-launchable)"
sed -i.bak "s|<launchable type=\"desktop-id\">$APP_ID.desktop</launchable>|<launchable type=\"desktop-id\">fermix-desktop.desktop</launchable>|" \
  "$repo/packaging/$APP_ID.metainfo.xml"
expect_refusal "a launchable that names a desktop entry nothing installs" "$repo"

repo="$(copy_repo drifted-bus-unit)"
sed -i.bak "s|BusName=$APP_ID|BusName=io.tezra.FermixDesktop|" \
  "$repo/packaging/systemd/app-$APP_ID.service"
expect_refusal "an activation unit taking a name nothing activates" "$repo"

repo="$(copy_repo missing-raster)"
rm "$repo/packaging/icons/hicolor/48x48/apps/$APP_ID.png"
expect_refusal "an icon size the package claims and does not carry" "$repo"

repo="$(copy_repo renamed-icon)"
mv "$repo/packaging/icons/hicolor/scalable/apps/$APP_ID.svg" \
  "$repo/packaging/icons/hicolor/scalable/apps/fermix.svg"
expect_refusal "a scalable icon named something the theme will not look up" "$repo"

repo="$(copy_repo missing-unit)"
rm "$repo/packaging/systemd/app-$APP_ID.service"
expect_refusal "a D-Bus service whose unit is not in the package" "$repo"

echo "check_app_identity_test: every refusal fired"

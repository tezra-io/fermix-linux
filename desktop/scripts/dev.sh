#!/usr/bin/env bash
# Fast development loop. Builds the app inside the GNOME 50 SDK (Rust extension)
# with incremental cargo caches kept in this directory, then runs the debug binary
# on the GNOME 50 runtime with the same sandbox permissions the manifest grants.
#   scripts/dev.sh build   # compile only
#   scripts/dev.sh run     # compile, then run against the host daemon
#   scripts/dev.sh check   # fmt, clippy -D warnings, tests (inside the SDK)
#   scripts/dev.sh fmt     # apply rustfmt
set -euo pipefail
here=$(cd "$(dirname "$0")/.." && pwd)
sdk=(flatpak run --share=network --filesystem="$here"
     --env=CARGO_HOME="$here/.cargo-sdk" --env=CARGO_TARGET_DIR="$here/target-sdk"
     --command=bash org.gnome.Sdk//50 -c)

in_sdk() { "${sdk[@]}" "source /usr/lib/sdk/rust-stable/enable.sh && cd '$here' && $1"; }

build() { in_sdk "cargo build -p fermix-desktop"; }

check() {
  in_sdk "cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace"
}

run() {
  build
  # FERMIX_HOME, when set, points the app at another daemon home (e.g. an empty one
  # to see the not-running screens without stopping the real daemon).
  local home_env=()
  [ -n "${FERMIX_HOME:-}" ] && home_env=(--env=FERMIX_HOME="$FERMIX_HOME")
  exec flatpak run "${home_env[@]}" --filesystem="$here/target-sdk:ro" --filesystem="$HOME/.fermix:ro" \
    --socket=wayland --socket=fallback-x11 --share=ipc --device=dri \
    --talk-name=org.freedesktop.systemd1 --system-talk-name=org.freedesktop.login1 \
    --filesystem=xdg-config/fermix:ro --socket=pulseaudio --own-name=io.tezra.Fermix --env=G_MESSAGES_DEBUG="${G_MESSAGES_DEBUG:-}" \
    --command="$here/target-sdk/debug/fermix-desktop" org.gnome.Platform//50 "$@"
}

case "${1:-run}" in
  build) build ;;
  fmt) in_sdk "cargo fmt --all" ;;
  check) check ;;
  run) shift || true; run "$@" ;;
  *) echo "usage: $0 build|run|check|fmt" >&2; exit 2 ;;
esac

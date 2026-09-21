#!/usr/bin/env bash
#
# The application opens no network connection, as a gate rather than a comment.
#
# The single package carries its own GLib, and a private GLib cannot load a host
# GIO module, so every module a stock GTK application gets for free has to be
# decided. glib-networking, which is what supplies GIO's TLS backend and its
# DNS resolver, is deliberately not bundled: everything this window shows goes
# over `daemon.sock`, and every outbound request belongs to the engine, which is
# a separate process with its own TLS.
#
# That claim is only worth what a machine can check it against, and the reason
# it needs a machine is that nothing else notices. It is worth being exact about
# the failure, because the obvious guess about it is wrong.
#
# The `g_tls_*` and `g_resolver_*` entry points are defined by libgio itself,
# not by glib-networking. glib-networking supplies the *backend* that implements
# them, as a loadable GIO module. So an application that calls one of them
# compiles, links, loads and starts perfectly: the symbol resolves against
# libgio like any other. What GLib does when no backend module is installed is
# substitute `GDummyTlsBackend`, whose `g_tls_backend_supports_tls` answers
# false and whose every operation returns an error saying TLS support is not
# available.
#
# There is therefore no link error, no load error and no missing-symbol message
# at any point. The first sign is a person's operation failing, at the moment
# they try it, on their machine. A development host is worse than no signal: it
# usually has glib-networking installed, so the same code path works there and
# the defect ships. Reading the symbols the binary names is the only place this
# can be caught before it reaches somebody.
#
# What is forbidden, and why each family is on the list:
#
#   g_tls_*, g_dtls_*        TLS and DTLS. libgio declares them; the backend
#                            that implements them is not shipped
#   g_resolver_*             DNS. The resolver is glib-networking's too
#   g_proxy_*                Proxy resolution, which goes through the same
#                            module and the same extension point
#   g_network_address_*      Name-based addresses and service lookups: both
#   g_network_service_*      resolve a host name before they connect
#   g_network_monitor_*      The connectivity monitor, another module
#   g_socket_client_*tls*    Turning a plain socket client into a TLS one
#
# What is deliberately allowed, and why:
#
#   g_socket_client_*        The management client dials `daemon.sock`, and the
#   g_unix_socket_address_*  setup assistant asks the engine's own web door on
#   g_socket_*               loopback whether the port it wants is already held.
#                            Both are local, neither resolves a name, and
#                            neither needs a GIO module at all
#
# Usage:
#   scripts/check_no_network.sh                 the built application binary
#   scripts/check_no_network.sh <path to ELF>   any ELF, for the staged tree
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

fail() {
  echo "check_no_network: $*" >&2
  exit 1
}

# Every family, as an extended regular expression anchored at the symbol's
# start, then `::`, then the sentence printed when it is found. `::` is the
# separator because a regular expression here uses `|` for alternation.
FORBIDDEN=(
  '^g_tls_|^g_dtls_::TLS, which this application has no backend for'
  '^g_resolver_::name resolution, which this application has no backend for'
  '^g_proxy_::proxy resolution, which goes through the module that is not shipped'
  '^g_network_address_|^g_network_service_::a name-based address, which resolves a host name before it connects'
  '^g_network_monitor_::the connectivity monitor, which is another module that is not shipped'
  '^g_socket_client_(set|get)_tls::a socket client asked to speak TLS'
)

binary_from_arguments() {
  [ $# -gt 1 ] && fail "one ELF at a time, and $# were given"
  if [ $# -eq 1 ]; then
    echo "$1"
    return
  fi

  # The build container and a host build put it in different places, and a
  # packaging run points at the staged tree with an argument instead.
  local candidate
  for candidate in \
    "${CARGO_TARGET_DIR:-}/debug/fermix-desktop" \
    "${CARGO_TARGET_DIR:-}/release/fermix-desktop" \
    "$ROOT_DIR/App/Fermix/target/debug/fermix-desktop" \
    "$ROOT_DIR/App/Fermix/target/release/fermix-desktop"; do
    if [ -n "$candidate" ] && [ -f "$candidate" ]; then
      echo "$candidate"
      return
    fi
  done

  fail "no application binary was found; build it first or name one"
}

# The dynamic symbols an ELF names but does not define, which is exactly the set
# it expects a shared library to answer for. `nm -D` is the reader; `objdump -T`
# is the fallback for an image that carries only binutils' other half.
undefined_symbols() {
  local binary="$1"
  if command -v nm >/dev/null 2>&1; then
    nm -D --undefined-only "$binary" | awk '{ print $NF }'
    return
  fi
  if command -v objdump >/dev/null 2>&1; then
    objdump -T "$binary" | awk '$2 == "*UND*" { print $NF }'
    return
  fi
  fail "neither nm nor objdump is installed, and one of them reads the symbols"
}

BINARY="$(binary_from_arguments "$@")"
[ -f "$BINARY" ] || fail "no ELF at $BINARY"

SYMBOLS="$(undefined_symbols "$BINARY" | sort -u)"
[ -n "$SYMBOLS" ] || fail "$BINARY names no dynamic symbol at all, so it was not read"

echo "check_no_network: $BINARY"

failures=0
for entry in "${FORBIDDEN[@]}"; do
  pattern="${entry%%::*}"
  reason="${entry#*::}"

  found="$(printf '%s\n' "$SYMBOLS" | grep -E "$pattern" || true)"
  if [ -n "$found" ]; then
    echo "check_no_network: the application names $reason:" >&2
    # Quoted and indented with sed rather than left to word splitting: a symbol
    # name cannot carry a space, but a gate that relies on that is one glob
    # character away from reading its own findings wrong.
    printf '%s\n' "$found" | sed 's/^/  /' >&2
    failures=$((failures + 1))
  fi
done

if [ "$failures" -gt 0 ]; then
  echo "check_no_network: the package bundles no glib-networking, so these symbols" >&2
  echo "  still link and still load: libgio declares them and GLib substitutes a" >&2
  echo "  dummy backend that refuses every operation. Nothing fails until a person" >&2
  echo "  tries it, on their machine. A development host with glib-networking" >&2
  echo "  installed runs the same code path successfully, which is why this is a" >&2
  echo "  gate and not a build error." >&2
  fail "$failures forbidden symbol famil(ies)"
fi

# Said out loud, so a reader of the log can see that the local paths are
# deliberate rather than overlooked.
local_sockets="$(printf '%s\n' "$SYMBOLS" | grep -E '^g_(socket|unix_socket)' || true)"
if [ -n "$local_sockets" ]; then
  echo "check_no_network: local sockets, which are allowed:"
  printf '%s\n' "$local_sockets" | sed 's/^/  /'
fi

echo "check_no_network: the application names no TLS, resolver or proxy symbol"

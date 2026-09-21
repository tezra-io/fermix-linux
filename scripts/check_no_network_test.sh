#!/usr/bin/env bash
#
# Exercise check_no_network.sh's refusals against ELFs written for the purpose.
#
# The gate exists because its failure mode is silent at build time and loud on
# somebody else's machine, so the gate itself has to be seen failing. Every case
# below is a real ELF compiled here and linked against the host's GIO, not a
# text fixture: what the gate reads is a dynamic symbol table, and a fixture
# that was not linked would prove nothing about the reader.
#
# It also runs the gate against the application's own binary when one has been
# built, which is the case that has to keep passing.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
GATE="$ROOT_DIR/scripts/check_no_network.sh"

WORK="$(mktemp -d "${TMPDIR:-/tmp}/check-no-network-test.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

fail() {
  echo "check_no_network_test: $*" >&2
  exit 1
}

# `cc` on a Debian-family image; the AlmaLinux build image installs the
# compiler as `gcc` and carries no `cc` of its own.
COMPILER=""
for candidate in cc gcc; do
  if command -v "$candidate" >/dev/null 2>&1; then
    COMPILER="$candidate"
    break
  fi
done
[ -n "$COMPILER" ] || fail "no C compiler, and every case here is a linked ELF"

command -v pkg-config >/dev/null 2>&1 || fail "no pkg-config, so GIO cannot be found"
pkg-config --exists gio-2.0 || fail "gio-2.0 is not installed, so nothing can be linked against it"

GIO_CFLAGS="$(pkg-config --cflags gio-2.0)"
GIO_LIBS="$(pkg-config --libs gio-2.0)"

# One ELF that calls exactly the named GIO functions, so the gate has a real
# symbol table to read. `volatile` keeps the calls from being optimised away.
build_elf() {
  local name="$1"
  shift

  local source="$WORK/$name.c"
  {
    echo '#include <gio/gio.h>'
    echo 'int main(void) {'
    echo '  volatile void *sink;'
    local call
    for call in "$@"; do
      echo "  sink = (void *) $call;"
    done
    echo '  (void) sink;'
    echo '  return 0;'
    echo '}'
  } >"$source"

  # shellcheck disable=SC2086
  "$COMPILER" $GIO_CFLAGS -o "$WORK/$name" "$source" $GIO_LIBS ||
    fail "$name did not compile, so this case proves nothing"
  echo "$WORK/$name"
}

expect_refusal() {
  local what="$1" elf="$2"
  if bash "$GATE" "$elf" >/dev/null 2>&1; then
    fail "$what was accepted"
  fi
  echo "  refused: $what"
}

expect_acceptance() {
  local what="$1" elf="$2"
  bash "$GATE" "$elf" >/dev/null || fail "$what was refused"
  echo "  accepted: $what"
}

echo "check_no_network_test: what the application is allowed to name"

# The two local paths the application really uses: the management client dialling
# `daemon.sock`, and the setup assistant asking the engine's web door on
# loopback whether its port is held. Neither resolves a name and neither needs a
# GIO module, and a gate that refused them would refuse the product.
expect_acceptance "a unix socket and a plain socket client" \
  "$(build_elf local-sockets \
    'g_unix_socket_address_new("/tmp/daemon.sock")' \
    'g_socket_client_new()')"

echo "check_no_network_test: the families that have no backend on a user's machine"

expect_refusal "a TLS connection" \
  "$(build_elf tls 'g_tls_client_connection_new(NULL, NULL, NULL)')"

expect_refusal "a name resolution" \
  "$(build_elf resolver 'g_resolver_get_default()')"

expect_refusal "a proxy resolution" \
  "$(build_elf proxy 'g_proxy_resolver_get_default()')"

expect_refusal "a name-based address, which resolves before it connects" \
  "$(build_elf network-address 'g_network_address_new("fermix.ai", 443)')"

expect_refusal "the connectivity monitor" \
  "$(build_elf network-monitor 'g_network_monitor_get_default()')"

expect_refusal "a socket client told to speak TLS" \
  "$(build_elf socket-client-tls \
    'g_socket_client_new()' \
    '(g_socket_client_set_tls(g_socket_client_new(), TRUE), (void *) 0)')"

echo "check_no_network_test: refusals that are not about symbols at all"

expect_refusal "an ELF that is not there" "$WORK/never-built"

printf 'not an ELF at all\n' >"$WORK/plain-text"
expect_refusal "a file with no symbol table" "$WORK/plain-text"

echo "check_no_network_test: the application itself"

# Built or not, depending on what ran before this. Where it is there, it is the
# case that matters, and where it is not, saying so is better than a green run
# that quietly checked nothing. The gate finds it for itself, with no argument,
# which is also how CI calls it.
if bash "$GATE" >/dev/null 2>&1; then
  echo "  accepted: the application binary, found by the gate itself"
elif bash "$GATE" 2>&1 | grep -q "no application binary was found"; then
  echo "  skipped: no application binary is built yet"
else
  bash "$GATE" >&2 || true
  fail "the application binary was refused"
fi

echo "check_no_network_test: every refusal fired"

#!/usr/bin/env bash
# Offline tests for nfpm_config.py: the rendered configuration declares the relations the map, the
# lock and the engine give it, lists every file and symbolic link of the staged tree with its mode,
# lists only the directories the package owns, and refuses every input it cannot render whole.
#   desktop/packaging/nfpm_config_test.sh
set -euo pipefail
shopt -s inherit_errexit

here=$(cd "$(dirname "$0")" && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
# The staged trees here are written with the modes a build gives them.
umask 022

fail() {
  echo "nfpm_config_test: $*" >&2
  exit 1
}

expect_refusal() {
  local what="$1" reason="$2" err="$work/stderr"
  shift 2
  if "$@" > /dev/null 2> "$err"; then
    fail "$what was accepted"
  fi
  grep -qF -- "$reason" "$err" || fail "$what was refused for another reason: $(cat "$err")"
  echo "  refused: $what"
}

expect_line() {
  grep -qxF -- "$2" "$1" || fail "$(basename "$1") has no line: $2"
}

expect_no_line() {
  if grep -qF -- "$2" "$1"; then
    fail "$(basename "$1") has a line it must not: $2"
  fi
}

# A staged tree in the package's shape, a few files deep.
make_root() {
  local root="$1"
  mkdir -p "$root/usr/bin" "$root/usr/lib/fermix-desktop/bin" "$root/usr/lib/fermix-desktop/lib/gio" \
    "$root/usr/share/doc/fermix-desktop" "$root/usr/share/doc/fermix" "$root/usr/share/applications" \
    "$root/usr/share/man/man1" "$root/usr/share/lintian/overrides"
  printf 'engine\n' > "$root/usr/bin/fermix"
  printf 'window\n' > "$root/usr/lib/fermix-desktop/bin/fermix-desktop"
  printf 'lib\n' > "$root/usr/lib/fermix-desktop/lib/libx.so.1.0"
  printf 'module\n' > "$root/usr/lib/fermix-desktop/lib/gio/libmodule.so"
  printf 'copyright\n' > "$root/usr/share/doc/fermix-desktop/copyright"
  printf '{}\n' > "$root/usr/share/doc/fermix-desktop/runtime-manifest.json"
  printf 'copyright\n' > "$root/usr/share/doc/fermix/copyright"
  printf '[Desktop Entry]\n' > "$root/usr/share/applications/io.tezra.Fermix.desktop"
  printf 'window page\n' > "$root/usr/share/man/man1/fermix-desktop.1.gz"
  printf 'engine page\n' > "$root/usr/share/man/man1/fermix.1.gz"
  printf 'fermix-desktop: a-tag\n' > "$root/usr/share/lintian/overrides/fermix-desktop"
  chmod 0755 "$root/usr/bin/fermix" "$root/usr/lib/fermix-desktop/bin/fermix-desktop"
  chmod 0644 "$root/usr/lib/fermix-desktop/lib/libx.so.1.0" "$root/usr/share/doc/fermix/copyright" \
    "$root/usr/share/doc/fermix-desktop/copyright" "$root/usr/share/doc/fermix-desktop/runtime-manifest.json" \
    "$root/usr/share/man/man1/fermix-desktop.1.gz" "$root/usr/share/man/man1/fermix.1.gz" \
    "$root/usr/share/lintian/overrides/fermix-desktop"
  ln -s libx.so.1.0 "$root/usr/lib/fermix-desktop/lib/libx.so.1"
  ln -s ../lib/fermix-desktop/bin/fermix-desktop "$root/usr/bin/fermix-desktop"
}

relations() {
  printf '{"deb": {"asset": "fermix_0.0.1_amd64.deb", "depends": %s},\n' "$2" > "$1"
  printf ' "rpm": {"asset": "fermix-0.0.1-1.x86_64.rpm", "requires": %s}}\n' "$3" >> "$1"
}

lock() {
  printf '{"glibc_floor": "2.34", "host_libraries": [%s]}\n' "$2" > "$1"
}

render() {
  local version="$1" arch="$2" out="$3"
  shift 3
  "$here/nfpm_config.py" --template "${TEMPLATE:-$here/nfpm-fermix-desktop.yaml.tmpl}" \
    --map "${MAP:-$work/map}" --lock "${LOCK:-$work/lock.json}" \
    --engine-relations "${RELATIONS:-$work/relations.json}" --root "${ROOT:-$work/root}" \
    --version "$version" --arch "$arch" --mtime 2026-10-04T00:00:00Z \
    --postinstall /scripts/postinstall.sh --postremove /scripts/postremove.sh --out "$out"
}

make_root "$work/root"
relations "$work/relations.json" '[]' '[]'
lock "$work/lock.json" '"libc.so.6", "ld-linux-x86-64.so.2", "libX11.so.6"'
cat > "$work/map" <<'EOF'
# a comment, and a blank line

libc.so.6 :: libc6 (>= {glibc_floor}) :: libc.so.6(GLIBC_{glibc_floor})(64bit)
ld-linux-x86-64.so.2 :: libc6 (>= {glibc_floor}) :: libc.so.6(GLIBC_{glibc_floor})(64bit)
libX11.so.6 :: libx11-6 :: libX11.so.6()(64bit)
@dconf :: dconf-service :: dconf
EOF

echo "nfpm_config_test: a staged tree, rendered"
render 0.0.1 amd64 "$work/nfpm.yaml" > /dev/null
out="$work/nfpm.yaml"
expect_line "$out" 'version: "0.0.1"'
expect_line "$out" 'arch: amd64'
expect_line "$out" 'mtime: "2026-10-04T00:00:00Z"'
expect_line "$out" '      - fermix (= 0.0.1)'
expect_line "$out" '      - fermix = 0.0.1'
expect_line "$out" '    depends: ["libc6 (>= 2.34)", "libx11-6", "dconf-service"]'
expect_line "$out" '    depends: ["libc.so.6(GLIBC_2.34)(64bit)", "libX11.so.6()(64bit)", "dconf"]'
expect_line "$out" '  postinstall: /scripts/postinstall.sh'
expect_line "$out" '  postremove: /scripts/postremove.sh'
expect_no_line "$out" '{{'
expect_no_line "$out" 'obsoletes'
echo "  ok: the version, the relations once each in the map's order, and the scripts"

root="$work/root"
expect_line "$out" "  - {\"dst\": \"/usr/bin/fermix\", \"file_info\": {\"mode\": 493}, \"src\": \"$root/usr/bin/fermix\", \"type\": \"file\"}"
expect_line "$out" "  - {\"dst\": \"/usr/lib/fermix-desktop/bin/fermix-desktop\", \"file_info\": {\"mode\": 493}, \"src\": \"$root/usr/lib/fermix-desktop/bin/fermix-desktop\", \"type\": \"file\"}"
expect_line "$out" "  - {\"dst\": \"/usr/share/doc/fermix/copyright\", \"file_info\": {\"mode\": 420}, \"src\": \"$root/usr/share/doc/fermix/copyright\", \"type\": \"file\"}"
expect_line "$out" '  - {"dst": "/usr/bin/fermix-desktop", "file_info": {"mode": 511}, "src": "../lib/fermix-desktop/bin/fermix-desktop", "type": "symlink"}'
expect_line "$out" '  - {"dst": "/usr/lib/fermix-desktop/lib/libx.so.1", "file_info": {"mode": 511}, "src": "libx.so.1.0", "type": "symlink"}'
# The package's own documents and manual page: a plain file in the deb, %doc in the rpm. The
# copyright file is the rpm's %license, under /usr/share/licenses/fermix-desktop as Fedora keeps
# licences, in a directory the rpm owns. The engine's stay as the fermix rpm has them, plain files
# in both. The lintian overrides are the deb's alone.
doc="$root/usr/share/doc/fermix-desktop"
man="$root/usr/share/man/man1"
expect_line "$out" "  - {\"dst\": \"/usr/share/doc/fermix-desktop/copyright\", \"file_info\": {\"mode\": 420}, \"packager\": \"deb\", \"src\": \"$doc/copyright\", \"type\": \"file\"}"
expect_line "$out" "  - {\"dst\": \"/usr/share/licenses/fermix-desktop/copyright\", \"file_info\": {\"mode\": 420}, \"packager\": \"rpm\", \"src\": \"$doc/copyright\", \"type\": \"license\"}"
expect_line "$out" '  - {"dst": "/usr/share/licenses/fermix-desktop", "file_info": {"mode": 493}, "packager": "rpm", "type": "dir"}'
expect_no_line "$out" "\"dst\": \"/usr/share/doc/fermix-desktop/copyright\", \"file_info\": {\"mode\": 420}, \"packager\": \"rpm\""
expect_line "$out" "  - {\"dst\": \"/usr/share/doc/fermix-desktop/runtime-manifest.json\", \"file_info\": {\"mode\": 420}, \"packager\": \"rpm\", \"src\": \"$doc/runtime-manifest.json\", \"type\": \"doc\"}"
expect_line "$out" "  - {\"dst\": \"/usr/share/man/man1/fermix-desktop.1.gz\", \"file_info\": {\"mode\": 420}, \"packager\": \"deb\", \"src\": \"$man/fermix-desktop.1.gz\", \"type\": \"file\"}"
expect_line "$out" "  - {\"dst\": \"/usr/share/man/man1/fermix-desktop.1.gz\", \"file_info\": {\"mode\": 420}, \"packager\": \"rpm\", \"src\": \"$man/fermix-desktop.1.gz\", \"type\": \"doc\"}"
expect_line "$out" "  - {\"dst\": \"/usr/share/man/man1/fermix.1.gz\", \"file_info\": {\"mode\": 420}, \"src\": \"$man/fermix.1.gz\", \"type\": \"file\"}"
expect_line "$out" "  - {\"dst\": \"/usr/share/lintian/overrides/fermix-desktop\", \"file_info\": {\"mode\": 420}, \"packager\": \"deb\", \"src\": \"$root/usr/share/lintian/overrides/fermix-desktop\", \"type\": \"file\"}"
[ "$(grep -c '"dst": "/usr/share/lintian/overrides/fermix-desktop"' "$out")" = 1 ] ||
  fail "the lintian overrides are listed for the rpm too"
for dir in /usr/lib/fermix-desktop /usr/lib/fermix-desktop/bin /usr/lib/fermix-desktop/lib \
  /usr/lib/fermix-desktop/lib/gio /usr/share/doc/fermix-desktop; do
  expect_line "$out" "  - {\"dst\": \"$dir\", \"file_info\": {\"mode\": 493}, \"type\": \"dir\"}"
done
for dir in /usr /usr/bin /usr/lib /usr/share /usr/share/doc /usr/share/doc/fermix /usr/share/applications; do
  expect_no_line "$out" "\"dst\": \"$dir\", "
done
entries="$(grep -c '^  - {' "$out")"
# Every file and link once, the package's two documents and its manual page once more, the five
# owned directories, and the rpm's licence directory.
want="$(( $(find "$root" -type f -o -type l | wc -l) + 3 + 5 + 1 ))"
[ "$entries" = "$want" ] || fail "$entries contents entries, and the tree has $want to list"
python3 -c 'import json, sys
for line in open(sys.argv[1]):
    if line.startswith("  - {"):
        json.loads(line[4:])' "$out" || fail "a contents entry is not one JSON object"
echo "  ok: $entries entries, every file and link with its mode, links 0777, the package's documents"
echo "      and manual page per packager, the lintian overrides in the deb alone, and only the owned"
echo "      directories"

echo "nfpm_config_test: a desktop-only rebuild, and the engine's own relations"
relations "$work/relations-declared.json" '["libyaml-0-2"]' '["libyaml-0.so.2()(64bit)"]'
RELATIONS="$work/relations-declared.json" render 0.0.1+2 amd64 "$work/rebuild.yaml" > /dev/null
expect_line "$work/rebuild.yaml" 'version: "0.0.1+2"'
expect_line "$work/rebuild.yaml" '      - fermix (= 0.0.1+2)'
expect_line "$work/rebuild.yaml" '    depends: ["libc6 (>= 2.34)", "libx11-6", "dconf-service", "libyaml-0-2"]'
expect_line "$work/rebuild.yaml" \
  '    depends: ["libc.so.6(GLIBC_2.34)(64bit)", "libX11.so.6()(64bit)", "dconf", "libyaml-0.so.2()(64bit)"]'
echo "  ok: carried after the host relations"

echo "nfpm_config_test: the checked-in map covers the checked-in lock exactly"
MAP="$here/host_relations.map" LOCK="$here/runtime/RUNTIME.lock.json" \
  render 0.12.1 amd64 "$work/real.yaml" > /dev/null
grep -q 'libc6 (>= 2.34)' "$work/real.yaml" || fail "the real map does not state the glibc floor"
grep -q '"libpulse0"' "$work/real.yaml" || fail "the real map does not declare libpulse0"
grep -q '"libstdc++.so.6()(64bit)"' "$work/real.yaml" || fail "the real map does not declare libstdc++"
echo "  ok: every host library has a row, and every row is a host library"

echo "nfpm_config_test: inputs it will not render"
expect_refusal "a version with a hyphen" "'-'" render 0.0.1-1 amd64 "$work/x.yaml"
expect_refusal "a version with an epoch" "':'" render 1:0.0.1 amd64 "$work/x.yaml"
expect_refusal "a version that is not X.Y.Z" "not a package version" render v0.0.1 amd64 "$work/x.yaml"
expect_refusal "a third architecture" "riscv64" render 0.0.1 riscv64 "$work/x.yaml"

lock "$work/lock-more.json" '"libc.so.6", "ld-linux-x86-64.so.2", "libX11.so.6", "libXnew.so.1"'
LOCK="$work/lock-more.json" expect_refusal "a host library the map has no row for" \
  "host_libraries names libXnew.so.1, which host_relations.map has no row for" \
  render 0.0.1 amd64 "$work/x.yaml"
lock "$work/lock-less.json" '"libc.so.6", "ld-linux-x86-64.so.2"'
LOCK="$work/lock-less.json" expect_refusal "a map row for a library the lock does not name" \
  "libX11.so.6, which the lock's host_libraries does not name" render 0.0.1 amd64 "$work/x.yaml"
printf 'libc.so.6 : libc6\n' >> "$work/map-bad" && cat "$work/map" >> "$work/map-bad"
MAP="$work/map-bad" expect_refusal "a malformed map row" "line 1" render 0.0.1 amd64 "$work/x.yaml"
printf '{"deb": {"asset": "fermix_0.0.1_amd64.deb", "depends": []}, "rpm": null}\n' > "$work/no-rpm.json"
RELATIONS="$work/no-rpm.json" expect_refusal "an engine stage with no rpm" "no rpm" \
  render 0.0.1 amd64 "$work/x.yaml"

cp -a "$work/root" "$work/no-engine" && rm "$work/no-engine/usr/bin/fermix"
ROOT="$work/no-engine" expect_refusal "a tree with no engine" "/usr/bin/fermix" render 0.0.1 amd64 "$work/x.yaml"
cp -a "$work/root" "$work/no-window" && rm "$work/no-window/usr/lib/fermix-desktop/bin/fermix-desktop"
ROOT="$work/no-window" expect_refusal "a tree with no window" "/usr/lib/fermix-desktop/bin/fermix-desktop" \
  render 0.0.1 amd64 "$work/x.yaml"
cp -a "$work/root" "$work/fifo" && mkfifo "$work/fifo/usr/share/doc/fermix-desktop/pipe"
ROOT="$work/fifo" expect_refusal "a tree holding a FIFO" "neither a file, a directory nor a symbolic link" \
  render 0.0.1 amd64 "$work/x.yaml"
cp -a "$work/root" "$work/absolute-link" && ln -s /etc/passwd "$work/absolute-link/usr/share/doc/fermix-desktop/link"
ROOT="$work/absolute-link" expect_refusal "an absolute symbolic link" "absolute" render 0.0.1 amd64 "$work/x.yaml"
cp -a "$work/root" "$work/writable" && chmod 0664 "$work/writable/usr/share/doc/fermix-desktop/copyright"
ROOT="$work/writable" expect_refusal "a group-writable file" "has mode 0664" render 0.0.1 amd64 "$work/x.yaml"
cp -a "$work/root" "$work/setuid" && chmod 4755 "$work/setuid/usr/lib/fermix-desktop/bin/fermix-desktop"
ROOT="$work/setuid" expect_refusal "a setuid window" "has mode 4755" render 0.0.1 amd64 "$work/x.yaml"

sed 's/{{MTIME}}/{{WHEN}}/' "$here/nfpm-fermix-desktop.yaml.tmpl" > "$work/unknown.tmpl"
TEMPLATE="$work/unknown.tmpl" expect_refusal "a template with a placeholder nobody fills" "{{WHEN}}" \
  render 0.0.1 amd64 "$work/x.yaml"
grep -v '{{POSTREMOVE}}' "$here/nfpm-fermix-desktop.yaml.tmpl" > "$work/short.tmpl"
TEMPLATE="$work/short.tmpl" expect_refusal "a template missing a placeholder" "{{POSTREMOVE}}" \
  render 0.0.1 amd64 "$work/x.yaml"
[ ! -e "$work/x.yaml" ] || fail "a refused render left a configuration behind"
expect_refusal "no arguments" "usage" "$here/nfpm_config.py"

echo "nfpm_config_test: ok"

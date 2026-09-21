#!/usr/bin/env bash
# Running a build container as the person who invoked it, not as root.
#
# Both container entry points bind-mount the repository at /workspace and write
# their output into it. A container running as root therefore leaves root-owned
# files in a working tree that other people share, which is a mess only a
# password can clear up, and it is the reason this file exists rather than each
# script passing --user and hoping.
#
# Running as an ordinary uid has one consequence worth stating: the named cargo
# volume is created by the daemon and owned by root, so a non-root container
# cannot write into it. The fix is to hand the volume to that uid once, in a
# short root container, before any build runs. It is idempotent and costs a
# fraction of a second on a warm volume.
#
# Sourced, not run.

# The --user flags a build container is given, as an array the caller expands.
# On the rare host where the daemon cannot map the caller, this is still the
# caller's own uid: getting it wrong means root-owned files, so it is not
# guessed.
container_user_flags() {
  printf '%s\n' "--user" "$(id -u):$(id -g)"
}

# Hand the cache volume to the invoking uid, so the non-root build can write to
# it. The image is the build image, so no second image is pulled for a chown.
container_cache_prepare() {
  local image="$1" volume="$2"
  docker volume create "$volume" >/dev/null
  docker run --rm -v "$volume:/cache" --user 0:0 "$image" \
    chown -R "$(id -u):$(id -g)" /cache ||
    return 1
}

# The rpm capability column of scripts/host_relations.map, on one line, for the
# build image's HOST_CAPABILITIES argument. One list, read by the dependency
# gate and by the image, so the machine the tests run on and the machine the
# package describes cannot drift apart.
host_capabilities() {
  local map="$1"
  [ -f "$map" ] || return 1
  awk -F ' :: ' '/^[^#]/ && NF >= 3 { print $3 }' "$map" | sort -u | tr '\n' ' '
}

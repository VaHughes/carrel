#!/usr/bin/env bash
# Repack a dist release archive as a Debian package.
#
#   build-deb.sh carrel-x86_64-unknown-linux-musl.tar.xz  [OUTDIR]
#   build-deb.sh carrel-aarch64-unknown-linux-musl.tar.xz [OUTDIR]
#
# Writes carrel_<version>-1_<amd64|arm64>.deb into OUTDIR (default
# target/debian). Needs cargo-deb on PATH; compiles nothing.
#
# The package is built from the ARCHIVE, not from a compile, so the binary in
# the .deb is the one the shell installer and `carrel-bin` ship, byte for byte,
# and every other file is the flattened `include` copy dist put next to it. The
# asset list is `[package.metadata.deb]` in crates/carrel/Cargo.toml; it names
# files under `target/release/`, which is where this script unpacks the
# archive, inside a private CARGO_TARGET_DIR so it can never overwrite a
# developer's own cross build.
#
# Only the musl archives are accepted. They are static, so the package needs
# no Depends at all and one .deb serves every supported Debian and Ubuntu; the
# gnu archives would carry a libc6 floor from the release container.

set -euo pipefail
cd "$(dirname "$0")/.."

die() { printf 'build-deb: %s\n' "$1" >&2; exit 1; }

[ $# -ge 1 ] || die "usage: build-deb.sh ARCHIVE.tar.xz [OUTDIR]"
archive=$(realpath "$1")
out=$(realpath -m "${2:-target/debian}")
[ -f "$archive" ] || die "no such archive: $1"
command -v cargo-deb >/dev/null || die "cargo-deb is not on PATH (cargo install cargo-deb --locked)"

name=$(basename "$archive")
case $name in
carrel-x86_64-unknown-linux-musl.tar.xz) triple=x86_64-unknown-linux-musl ;;
carrel-aarch64-unknown-linux-musl.tar.xz) triple=aarch64-unknown-linux-musl ;;
*) die "$name is not a musl Linux release archive (x86_64 or aarch64)" ;;
esac

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
stage="$work/target/$triple/release"
mkdir -p "$stage"
# dist wraps everything in one top-level directory; drop it, which leaves the
# same flat names `include` produced.
tar xf "$archive" -C "$stage" --strip-components=1

# Every asset the metadata names, read from the metadata itself. `carrel.1.gz`
# is made below from `carrel.1`, so it is the uncompressed page that must be in
# the archive.
assets=$(grep -oE '"target/release/[^"]+"' crates/carrel/Cargo.toml \
  | sed -E 's|"target/release/||; s|"$||; s|\.1\.gz$|.1|' | sort -u)
[ -n "$assets" ] || die "no target/release/ assets in crates/carrel/Cargo.toml — did its shape change?"
for f in $assets; do
  [ -f "$stage/$f" ] || die "$name has no $f — has dist-workspace.toml's include list changed?"
done

# The tree and the archive must be the same release: the version on the .deb
# comes from Cargo.toml, the contents from the archive. The man page's .TH line
# is stamped with the version (check-packaging.sh enforces it), and unlike the
# binary it can be read on any architecture.
ws_ver=$(awk '/^\[workspace\.package\]/{f=1; next} /^\[/{f=0}
              f && /^version = /{gsub(/["[:space:]]/, "", $3); print $3; exit}' Cargo.toml)
man_ver=$(sed -n 's/^\.TH .* "carrel \([^"]*\)".*/\1/p' "$stage/carrel.1" | head -1)
[ "$man_ver" = "$ws_ver" ] \
  || die "$name is carrel ${man_ver:-?}, this checkout is $ws_ver — check out the matching tag"

# Debian policy 12.1: man pages compressed with `gzip -9n` (no name, no
# timestamp, so the result is reproducible). Done here rather than left to
# cargo-deb, which skips compression for anything under the target directory.
gzip -9n "$stage/carrel.1"

# A stable timestamp for every member, so rebuilding from the same archive
# gives the same .deb: the binary's own mtime, as dist recorded it.
export SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH:-$(stat -c %Y "$stage/carrel")}

mkdir -p "$out"
# --no-strip: dist already stripped it, and a host `strip` cannot read the
# other architecture anyway. --deb-revision 1 is spelled out because it is
# part of the version users see.
CARGO_TARGET_DIR="$work/target" cargo-deb \
  -p carrel --target "$triple" --no-build --no-strip --no-dbgsym \
  --deb-revision 1 --output "$out"

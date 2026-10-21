#!/usr/bin/env bash
# Stamp the post-tag checksums into the packaging recipes.
#
#   scripts/stamp-release.sh VERSION TARBALL_SHA256 X86_64_SHA256 AARCH64_SHA256 \
#                            [NIX_SRC_HASH NIX_CARGO_HASH]
#
# A checksum cannot exist before the artifact it sums, so this is the one part
# of a release that has to follow the tag. Versions are NOT touched: they go in
# the release commit, before the tag, and check-packaging.sh holds every recipe
# to Cargo.toml's. This script refuses to stamp recipes that name a different
# version, because that means main has already moved on to the next release.
#
# publish.yml runs it after every Release run; it can be run by hand with the
# same arguments. The sums are the tag tarball's (PKGBUILD-carrel and its
# .SRCINFO), the two musl archives' (PKGBUILD-carrel-bin), and the two SRI
# hashes nix printed (carrel-package.nix).
#
# .SRCINFO is edited in place rather than regenerated with makepkg, which the
# CI runner does not have. Only its sha256sums line changes, and
# check-packaging.sh fails if that line disagrees with the PKGBUILD's.
set -euo pipefail

[ $# -eq 4 ] || [ $# -eq 6 ] || {
  echo "usage: $0 VERSION TARBALL_SHA256 X86_64_SHA256 AARCH64_SHA256 [NIX_SRC NIX_CARGO]" >&2
  exit 2
}
version=$1 tar_sum=$2 x86_sum=$3 arm_sum=$4
nix_src=${5:-} nix_cargo=${6:-}

cd "$(dirname "$0")/../contrib/packaging"

for s in "$tar_sum" "$x86_sum" "$arm_sum"; do
  [[ $s =~ ^[0-9a-f]{64}$ ]] || { echo "not a sha256: '$s'" >&2; exit 1; }
done
for s in "$nix_src" "$nix_cargo"; do
  [ -z "$s" ] || [[ $s =~ ^sha256-[A-Za-z0-9+/]{43}=$ ]] || { echo "not an SRI sha256: '$s'" >&2; exit 1; }
done

for f in PKGBUILD-carrel PKGBUILD-carrel-bin; do
  v=$(sed -n 's/^pkgver=//p' "$f")
  [ "$v" = "$version" ] || { echo "$f is at $v, not $version; refusing to stamp" >&2; exit 3; }
done
v=$(sed -n 's/^[[:space:]]*version = "\([^"]*\)".*/\1/p' carrel-package.nix | head -1)
[ "$v" = "$version" ] || { echo "carrel-package.nix is at $v, not $version; refusing to stamp" >&2; exit 3; }

sed -i "s/^sha256sums=('[0-9a-f]*')/sha256sums=('$tar_sum')/" PKGBUILD-carrel
sed -i "s/^\tsha256sums = [0-9a-f]*$/\tsha256sums = $tar_sum/" SRCINFO-carrel
sed -i "s/^sha256sums_x86_64=('[0-9a-f]*')/sha256sums_x86_64=('$x86_sum')/" PKGBUILD-carrel-bin
sed -i "s/^sha256sums_aarch64=('[0-9a-f]*')/sha256sums_aarch64=('$arm_sum')/" PKGBUILD-carrel-bin

if [ -n "$nix_src" ]; then
  # `hash` is the fetchFromGitHub one (the only `hash =` in the file);
  # `cargoHash` is matched separately. Either may be lib.fakeHash or a stale
  # hash from the previous release.
  sed -i -E "s|^([[:space:]]*hash = ).*;|\1\"$nix_src\";|" carrel-package.nix
  sed -i -E "s|^([[:space:]]*cargoHash = ).*;|\1\"$nix_cargo\";|" carrel-package.nix
fi

# Every edit must have landed: a recipe whose shape changed would otherwise be
# left unstamped with nothing said.
grep -q "^sha256sums=('$tar_sum')" PKGBUILD-carrel
grep -q "^	sha256sums = $tar_sum$" SRCINFO-carrel
grep -q "^sha256sums_x86_64=('$x86_sum')" PKGBUILD-carrel-bin
grep -q "^sha256sums_aarch64=('$arm_sum')" PKGBUILD-carrel-bin
if [ -n "$nix_src" ]; then
  grep -q "hash = \"$nix_src\";" carrel-package.nix
  grep -q "cargoHash = \"$nix_cargo\";" carrel-package.nix
fi
echo "stamped $version"

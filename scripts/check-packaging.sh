#!/usr/bin/env bash
# Will what we publish still resolve for the person downstream?
#
# Three checks: the recipes against what actually ships, .SRCINFO against
# its PKGBUILD, and the README's links as crates.io will rewrite them.
#
# `carrel-bin` on the AUR unpacks a RELEASE ARCHIVE and installs files out of
# it. Nothing else in CI builds that recipe, so until 2026-08-16 it installed
# five files that were not in the archive at all — `dist-workspace.toml`
# shipped only the desktop entry — and it would have failed on the first AUR
# build, in front of a user. It was found by hand-downloading a tarball.
#
# This is that check, automated. Two modes:
#
#   check-packaging.sh                  static: cross-check the recipe against
#                                       the dist `include` list. Runs in CI on
#                                       every push; needs no network.
#   check-packaging.sh ARCHIVE.tar.xz   verify against a REAL artifact. Run
#                                       this before publishing a binary
#                                       package (RELEASING.md step 8).
#
# NOTE: dist FLATTENS every `include` entry to the archive root, so
# `contrib/carrel.1` arrives as `carrel.1`. Recipes must use the flat names.

set -uo pipefail
cd "$(dirname "$0")/.."

RECIPE=contrib/packaging/PKGBUILD-carrel-bin
DIST=dist-workspace.toml
fail=0

note() { printf '  \033[31m✗\033[0m %s\n' "$1"; fail=1; }
ok() { printf '  \033[32m✓\033[0m %s\n' "$1"; }

# Every `install -Dm... "$dir/NAME"` the recipe reads out of the archive.
wanted=$(grep -oE 'install -Dm[0-9]+ "\$dir/[^"]+"' "$RECIPE" \
  | sed -E 's/.*\$dir\///; s/"$//' | sort -u)

if [ -z "$wanted" ]; then
  note "$RECIPE installs nothing from the archive — did its shape change?"
  echo
  exit 1
fi

if [ $# -ge 1 ]; then
  # --- real artifact ---
  archive=$1
  echo "carrel-bin against $archive"
  listing=$(tar tf "$archive" 2>/dev/null) || {
    note "cannot read $archive"
    exit 1
  }
  # Strip the single top-level directory dist wraps everything in.
  present=$(printf '%s\n' "$listing" | sed -E 's|^[^/]+/||' | grep -v '^$' | sort -u)
else
  # --- static cross-check ---
  echo "carrel-bin against $DIST's include list"
  # dist always ships the binary and the package's readme/licences; `include`
  # adds the rest, flattened to the archive root.
  included=$(sed -n '/^include = \[/,/^\]/p' "$DIST" \
    | grep -oE '"[^"]+"' | tr -d '"' | xargs -r -n1 basename)
  present=$(printf '%s\ncarrel\nREADME.md\nCHANGELOG.md\nLICENSE-MIT\nLICENSE-APACHE\n' \
    "$included" | grep -v '^$' | sort -u)
fi

while IFS= read -r f; do
  [ -z "$f" ] && continue
  if printf '%s\n' "$present" | grep -qxF "$f"; then
    ok "$f"
  else
    note "$f — the recipe installs it, the archive will not contain it"
  fi
done <<<"$wanted"

# The source recipes build from the git tarball, so they see the whole repo;
# the only thing to check is that the paths they name still exist.
echo
echo "source recipes against the working tree"
for f in $(grep -ohE 'install -Dm[0-9]+ (contrib|LICENSE)[^ ]*' \
  contrib/packaging/PKGBUILD-carrel contrib/packaging/carrel.spec \
  | awk '{print $3}' | sed 's|%{buildroot}.*||' | sort -u); do
  [ -z "$f" ] && continue
  if [ -e "$f" ]; then ok "$f"; else note "$f — named by a recipe, not in the tree"; fi
done

# --- every recipe against the workspace version ---
#
# The .SRCINFO check below compares two recipes to EACH OTHER, so it stays
# quiet whenever both are stale together, and nothing compared the spec or the
# nix expression to anything at all. `carrel-package.nix` sat at 2026.8.17
# while the workspace said 2026.8.31 — six stamp commits walked past it, and no
# check here could have caught that, because none of them ever looked at the
# file. `[workspace.package] version` in Cargo.toml is the one source of truth;
# measure every recipe against it, BY NAME, so that a recipe added later and
# then forgotten fails loudly rather than drifting quietly.
#
# The consequence for the release flow: the VERSION stamp belongs in the
# release commit, *before* the tag. Only the sha256sums have to wait, because
# they check a tarball GitHub does not serve until the tag exists. Stamping the
# version after the tag leaves the tagged tree — the tree release.yml's gate
# now runs this script against — naming the previous release, which is
# precisely the drift this check refuses.
echo
echo "recipe versions against Cargo.toml"
ws_ver=$(awk '/^\[workspace\.package\]/{f=1; next} /^\[/{f=0}
              f && /^version = /{gsub(/["[:space:]]/, "", $3); print $3; exit}' Cargo.toml)

recipe_version() {
  case $1 in
  *carrel.spec) sed -n 's/^Version:[[:space:]]*\([^[:space:]]*\).*/\1/p' "$1" | head -1 ;;
  *PKGBUILD-*) sed -n 's/^pkgver=\(.*\)$/\1/p' "$1" | head -1 ;;
  *SRCINFO-*) awk -F'= ' '/^\tpkgver = /{print $2; exit}' "$1" ;;
  *.nix) sed -n 's/^[[:space:]]*version = "\([^"]*\)".*/\1/p' "$1" | head -1 ;;
  esac
}

if [ -z "$ws_ver" ]; then
  note "cannot read [workspace.package] version from Cargo.toml"
else
  for recipe in contrib/packaging/carrel.spec \
    contrib/packaging/PKGBUILD-carrel \
    contrib/packaging/PKGBUILD-carrel-bin \
    contrib/packaging/carrel-package.nix \
    contrib/packaging/SRCINFO-carrel; do
    name=$(basename "$recipe")
    v=$(recipe_version "$recipe")
    if [ -z "$v" ]; then
      note "$name — no version found in it; did its shape change?"
    elif [ "$v" != "$ws_ver" ]; then
      note "$name says $v, Cargo.toml says $ws_ver — stamp the recipes in the release commit, before the tag"
    else
      ok "$name $v"
    fi
  done

  # The man page's .TH line is the version in the footer of every `man carrel`,
  # and it ships in the release archive alongside the binary that prints its
  # own. A version added by hand and checked by nothing is the recipe drift
  # again in a different file, so it is stamped and checked with the recipes.
  man_ver=$(sed -n 's/^\.TH .* "carrel \([^"]*\)".*/\1/p' contrib/carrel.1 | head -1)
  if [ -z "$man_ver" ]; then
    note "contrib/carrel.1 — the .TH line names no version"
  elif [ "$man_ver" != "$ws_ver" ]; then
    note "contrib/carrel.1 says $man_ver, Cargo.toml says $ws_ver"
  else
    ok "carrel.1 $man_ver"
  fi
fi

# --- .SRCINFO against its PKGBUILD ---
#
# `.SRCINFO` is a FLATTENED copy of the PKGBUILD: makepkg expands
# `$pkgname-$pkgver` when it generates the file, so hand-editing `pkgver` alone
# leaves the `source` line pointing at the previous tag. That is exactly what
# happened between 2026.8.21 and 2026.8.26 — pkgver said 2026.8.26, the source
# line fetched the v2026.8.21 tarball, and the sha256 was 2026.8.26's, so AUR
# would have failed the integrity check on the first build. Regenerate with
# `makepkg --printsrcinfo > contrib/packaging/SRCINFO-carrel`, never by hand.
# This check needs no makepkg, so it runs on any CI runner.
echo
echo ".SRCINFO against its PKGBUILD"
srcinfo=contrib/packaging/SRCINFO-carrel
pkgbuild=contrib/packaging/PKGBUILD-carrel
si_ver=$(awk -F'= ' '/^\tpkgver = /{print $2; exit}' "$srcinfo")
pb_ver=$(awk -F= '/^pkgver=/{print $2; exit}' "$pkgbuild")
si_sum=$(awk -F'= ' '/^\tsha256sums = /{print $2; exit}' "$srcinfo")
pb_sum=$(sed -n "s/^sha256sums=('\([a-f0-9]*\)').*/\1/p" "$pkgbuild" | head -1)

if [ -z "$si_ver" ] || [ -z "$pb_ver" ]; then
  note "cannot read pkgver from $srcinfo or $pkgbuild"
elif [ "$si_ver" != "$pb_ver" ]; then
  note "pkgver: .SRCINFO says $si_ver, PKGBUILD says $pb_ver"
else
  ok "pkgver $si_ver matches"
fi

# The expansion the flattening gets wrong.
if printf '%s\n' "$si_ver" | grep -q .; then
  if awk '/^\tsource = /' "$srcinfo" | grep -qF "v$si_ver.tar.gz" \
     && awk '/^\tsource = /' "$srcinfo" | grep -qF "carrel-$si_ver.tar.gz"; then
    ok "source line names v$si_ver"
  else
    note ".SRCINFO source line does not name v$si_ver — regenerate with makepkg --printsrcinfo"
  fi
fi

if [ -n "$si_sum" ] && [ "$si_sum" != "$pb_sum" ]; then
  note "sha256sums: .SRCINFO and PKGBUILD disagree"
elif [ -n "$si_sum" ]; then
  ok "sha256sums match"
fi

# --- the README's links, as crates.io will resolve them ---
#
# `cargo package` records `path_in_vcs` (here `crates/carrel`) in
# `.cargo_vcs_info.json`, and crates.io resolves every RELATIVE link in the
# rendered readme against `repository` + that path. Our readme lives at the
# REPO ROOT and is inherited by both crates, so a relative link lands one
# directory too deep and 404s: `assets/demo.gif` was served as
# `.../crates/carrel/assets/demo.gif` from launch until 2026-08-27, which is
# why both logo and demo were broken on the crate page while rendering fine
# on GitHub. Absolute URLs are the only form correct in both places.
echo
echo "README links as crates.io will resolve them"
readme_bad=$(grep -nP '(\]\(|<img[^>]*\ssrc=")(?!https?:|#|mailto:)' README.md || true)
if [ -n "$readme_bad" ]; then
  while IFS= read -r line; do
    note "README.md:${line%%:*} — relative link; crates.io resolves it under crates/carrel/"
  done <<<"$readme_bad"
else
  ok "every README link and image is absolute"
fi

echo
if [ "$fail" -eq 0 ]; then
  echo "Packaging recipes match what ships, and the README reads right off the repo."
else
  echo "Something above would break for someone downstream — see the ✗ lines."
fi
exit "$fail"

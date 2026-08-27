#!/usr/bin/env bash
# Do the packaging recipes install files that will actually be there?
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

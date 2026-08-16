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

echo
if [ "$fail" -eq 0 ]; then
  echo "Packaging recipes match what ships."
else
  echo "Packaging recipes would install files that will not exist."
fi
exit "$fail"

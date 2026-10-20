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
#   check-packaging.sh PACKAGE.deb      verify a built .deb (scripts/
#                                       build-deb.sh) against its metadata.
#                                       deb.yml runs it before uploading.
#
# The .deb is the same kind of recipe as `carrel-bin` — a repack of the
# release archive — so its asset list (`[package.metadata.deb]` in
# crates/carrel/Cargo.toml) is checked against the same archive contents.
#
# NOTE: dist FLATTENS every `include` entry to the archive root, so
# `contrib/carrel.1` arrives as `carrel.1`. Recipes must use the flat names.

set -uo pipefail
cd "$(dirname "$0")/.."

RECIPE=contrib/packaging/PKGBUILD-carrel-bin
DIST=dist-workspace.toml
DEBMETA=crates/carrel/Cargo.toml
fail=0

deb=
if [ $# -ge 1 ] && [[ $1 == *.deb ]]; then
  deb=$1
  shift
fi

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

# --- the .deb's assets, against the same archive contents ---
#
# Each asset is `["target/release/NAME", "DEST", "MODE"]`, where NAME is the
# flat archive name; build-deb.sh makes `carrel.1.gz` from `carrel.1`.
echo
echo ".deb assets against the same archive contents"
deb_assets=$(sed -n '/^\[package\.metadata\.deb\]/,/^\[/p' "$DEBMETA" \
  | grep -oE '^ *\["target/release/[^"]+", *"[^"]+", *"[0-7]+"\]' \
  | sed -E 's/^ *\["target\/release\/([^"]+)", *"([^"]+)", *"([0-7]+)"\]/\1 \2 \3/')
if [ -z "$deb_assets" ]; then
  note "$DEBMETA has no [package.metadata.deb] target/release/ assets — did its shape change?"
else
  while read -r src _ _; do
    f=${src%.1.gz}
    [ "$f" != "$src" ] && f=$f.1
    if printf '%s\n' "$present" | grep -qxF "$f"; then
      ok "$f"
    else
      note "$f — the .deb packs it, the archive will not contain it"
    fi
  done <<<"$deb_assets"
fi

# --- a built .deb, against its metadata ---
if [ -n "$deb" ]; then
  echo
  echo "$(basename "$deb") against $DEBMETA"
  t=$(mktemp -d)
  trap 'rm -rf "$t"' EXIT
  # dpkg-deb where it exists (every Debian/Ubuntu runner); `ar` elsewhere.
  if command -v dpkg-deb >/dev/null; then
    dpkg-deb --ctrl-tarfile "$deb" >"$t/control.tar" 2>/dev/null \
      && dpkg-deb --fsys-tarfile "$deb" >"$t/data.tar" 2>/dev/null
  else
    members=$(ar t "$deb" 2>/dev/null) \
      && ar p "$deb" "$(grep '^control\.tar' <<<"$members")" >"$t/control.tar" \
      && ar p "$deb" "$(grep '^data\.tar' <<<"$members")" >"$t/data.tar"
  fi || {
    note "cannot read $deb"
    exit 1
  }
  cmember=$(tar tf "$t/control.tar" | grep -E '^(\./)?control$' | head -1)
  control=$(tar xOf "$t/control.tar" "$cmember" 2>/dev/null)
  field() { sed -n "s/^$1: *//p" <<<"$control" | head -1; }

  ws_ver=$(awk '/^\[workspace\.package\]/{f=1; next} /^\[/{f=0}
                f && /^version = /{gsub(/["[:space:]]/, "", $3); print $3; exit}' Cargo.toml)
  [ "$(field Package)" = carrel ] && ok "Package: carrel" || note "Package is '$(field Package)', not carrel"
  [ "$(field Version)" = "$ws_ver-1" ] && ok "Version: $ws_ver-1" \
    || note "Version is '$(field Version)', Cargo.toml says $ws_ver (expected $ws_ver-1)"
  arch=$(field Architecture)
  case $arch in
  amd64 | arm64) ok "Architecture: $arch" ;;
  *) note "Architecture is '$arch' — the musl archives make amd64 and arm64 only" ;;
  esac
  # The binary is static musl. A dependency here means a dynamic binary got
  # packaged, which would install and then fail to start on an older libc.
  if [ -z "$(field Depends)" ]; then ok "no Depends (static binary)"; else note "Depends: $(field Depends) — a static binary needs none"; fi

  # Q21: carrel must never make itself the default markdown handler. A
  # maintainer script is the only place a .deb could do that (xdg-mime, a
  # mimeapps.list write); the desktop database is refreshed by
  # desktop-file-utils' own dpkg trigger without one.
  scripts=$(tar tf "$t/control.tar" | grep -E '(pre|post)(inst|rm)$|config$' || true)
  if [ -z "$scripts" ]; then ok "no maintainer scripts"; else note "maintainer scripts present: $scripts"; fi

  listing=$(tar --numeric-owner -tvf "$t/data.tar")
  if grep -qE 'mimeapps\.list|/etc/' <<<"$listing"; then
    note "the package writes under /etc or ships a mimeapps.list"
  fi

  while read -r src dest mode; do
    [[ $dest == */ ]] && dest=$dest$(basename "$src")
    perm=$(awk -v p="./$dest" '$NF == p {print $1, $2}' <<<"$listing")
    want=$( [ "$mode" = 755 ] && echo '-rwxr-xr-x' || echo '-rw-r--r--')
    if [ -z "$perm" ]; then
      note "$dest — in the metadata, not in the package"
    elif [ "$perm" != "$want 0/0" ]; then
      note "$dest is $perm, wanted $want 0/0"
    else
      ok "$dest"
    fi
  done <<<"$deb_assets"

  # Debian policy 12.1: man pages `gzip -9n`. The -n is what keeps the build
  # reproducible: no FNAME flag (bit 3 of byte 3), no MTIME (bytes 4-7); the
  # -9 is XFL = 2 (byte 8).
  man=$(tar xOf "$t/data.tar" ./usr/share/man/man1/carrel.1.gz 2>/dev/null | od -An -tu1 -N9 | tr -s ' ')
  if [ "$man" = " 31 139 8 0 0 0 0 0 2" ]; then
    ok "carrel.1.gz is gzip -9 with no name and no timestamp"
  else
    note "carrel.1.gz header is '$man' — expected gzip -9n output"
  fi
fi

# The source recipes build from the git tarball, so they see the whole repo;
# the only thing to check is that the paths they name still exist.
echo
echo "source recipes against the working tree"
for f in $(grep -ohE 'install -Dm[0-9]+ (contrib|LICENSE|THIRD-PARTY-NOTICES)[^ ]*' \
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

# --- the third-party notices, everywhere the binary goes ---
#
# Every carrel binary contains carrel-core's hyphenation data, which is under
# MIT, BSD-3-Clause and all-permissive notices (THIRD-PARTY-NOTICES has the
# texts). BSD-3-Clause's binary clause wants its notice "in the documentation
# and/or other materials provided with the distribution", so a recipe that
# installs the binary without this file ships it out of terms — and include_str!
# alone puts the notice in the binary, where nobody reads it. Each recipe is
# checked BY NAME, as with the versions above, so a new one cannot quietly skip
# it; and each states the same licence expression as Cargo.toml.
echo
echo "third-party notices in every recipe"
notices=THIRD-PARTY-NOTICES
ws_license=$(awk '/^\[workspace\.package\]/{f=1; next} /^\[/{f=0}
                  f && /^license = /{sub(/^license = "/, ""); sub(/"[[:space:]]*$/, ""); print; exit}' Cargo.toml)

if [ -s "$notices" ]; then ok "$notices at the repository root"; else note "$notices is missing from the repository root"; fi
if sed -n '/^include = \[/,/\]/p' "$DIST" | grep -qF "\"$notices\""; then
  ok "$DIST includes $notices (flattened to the archive root)"
else
  note "$DIST does not include $notices — no release archive, carrel-bin or .deb would carry it"
fi
# crates.io: `cargo package` takes a crate's directory only, so each crate
# carries the notices and both licences as symlinks to the root copies, which
# cargo packages as ordinary files.
for crate in crates/carrel-core crates/carrel; do
  for f in "$notices" LICENSE-MIT LICENSE-APACHE; do
    if [ -s "$crate/$f" ]; then ok "$crate/$f"; else note "$crate/$f is missing — the .crate on crates.io would not carry it"; fi
  done
done

has_line() { # FILE FIXED-STRING WHAT
  if grep -qF -- "$2" "$1"; then ok "$(basename "$1"): $3"; else note "$(basename "$1") does not $3"; fi
}
has_line contrib/packaging/PKGBUILD-carrel \
  'install -Dm644 THIRD-PARTY-NOTICES "$pkgdir/usr/share/licenses/$pkgname/THIRD-PARTY-NOTICES"' \
  "install $notices to /usr/share/licenses"
has_line contrib/packaging/PKGBUILD-carrel-bin \
  'install -Dm644 "$dir/THIRD-PARTY-NOTICES" "$pkgdir/usr/share/licenses/$pkgname/THIRD-PARTY-NOTICES"' \
  "install $notices to /usr/share/licenses"
if grep -qE "^%license([[:space:]].*)?[[:space:]]$notices([[:space:]]|$)" contrib/packaging/carrel.spec; then
  ok "carrel.spec: %license $notices"
else
  note "carrel.spec does not list $notices under %license"
fi
has_line contrib/packaging/carrel-package.nix \
  "install -Dm644 $notices \$out/share/doc/carrel/$notices" "install $notices to share/doc"
if printf '%s\n' "$deb_assets" | grep -qxF "$notices usr/share/doc/carrel/$notices 644"; then
  ok ".deb assets: $notices to /usr/share/doc/carrel"
else
  note ".deb assets do not install $notices to /usr/share/doc/carrel"
fi

if [ -z "$ws_license" ]; then
  note "cannot read [workspace.package] license from Cargo.toml"
else
  spec_license=$(sed -n 's/^License:[[:space:]]*//p' contrib/packaging/carrel.spec | head -1)
  [ "$spec_license" = "$ws_license" ] && ok "carrel.spec License matches Cargo.toml" \
    || note "carrel.spec License is '$spec_license', Cargo.toml says '$ws_license'"
  for pb in contrib/packaging/PKGBUILD-carrel contrib/packaging/PKGBUILD-carrel-bin; do
    pl=$(sed -n "s/^license=('\(.*\)')\$/\1/p" "$pb" | head -1)
    [ "$pl" = "$ws_license" ] && ok "$(basename "$pb") license matches Cargo.toml" \
      || note "$(basename "$pb") license is '$pl', Cargo.toml says '$ws_license'"
  done
  sl=$(awk -F' = ' '/^\tlicense = /{print $2; exit}' contrib/packaging/SRCINFO-carrel)
  [ "$sl" = "$ws_license" ] && ok "SRCINFO-carrel license matches Cargo.toml" \
    || note "SRCINFO-carrel license is '$sl', Cargo.toml says '$ws_license' — regenerate it"
  # nixpkgs spells licences its own way and has no FSFAP-no-warranty-disclaimer;
  # require one entry per licence family the expression names.
  nix_lic=$(sed -n '/license = with lib.licenses; \[/,/\];/p' contrib/packaging/carrel-package.nix)
  nix_missing=
  for want in mit asl20 bsd3 fsfap knuth; do
    printf '%s\n' "$nix_lic" | grep -qxE "[[:space:]]*$want" || nix_missing="$nix_missing $want"
  done
  if [ -z "$nix_missing" ]; then ok "carrel-package.nix meta.license covers the expression"
  else note "carrel-package.nix meta.license lacks:$nix_missing"; fi
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

# --- in-workspace dependencies are exact pins ---
#
# CalVer puts the year in the semver MAJOR, so a caret requirement such as
# `carrel-core = "2026.10.8"` accepts every 2026 core. An unlocked
# `cargo install carrel --version 2026.9.23` then resolved carrel-core
# 2026.10.8 and failed to compile — the merman sibling trap again, on our own
# crate. The two crates release in lockstep, so the frontend must name exactly
# the core it was built against.
echo
echo "in-workspace dependencies against Cargo.toml"
pins=$(grep -hE '^carrel-core[[:space:]]*=.*path' crates/*/Cargo.toml)
if [ -z "$pins" ]; then
  note "no carrel-core path dependency found — did its shape change?"
fi
while IFS= read -r line; do
  [ -z "$line" ] && continue
  req=$(printf '%s\n' "$line" | sed -nE 's/.*version[[:space:]]*=[[:space:]]*"([^"]*)".*/\1/p')
  if [ "$req" = "=$ws_ver" ]; then
    ok "carrel-core = \"$req\""
  else
    note "carrel-core requires \"$req\"; want \"=$ws_ver\" (exact — CalVer makes a caret span the year)"
  fi
done <<<"$pins"

echo
if [ "$fail" -eq 0 ]; then
  echo "Packaging recipes match what ships, and the README reads right off the repo."
else
  echo "Something above would break for someone downstream — see the ✗ lines."
fi
exit "$fail"

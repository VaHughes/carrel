# Packaging templates

**All five recipes are stamped for v2026.9.1**, and `scripts/check-packaging.sh` now
asserts every one of them against `[workspace.package] version` in `Cargo.toml`.
`carrel-package.nix` had not been touched since the commit that added it, and six stamp
commits walked straight past it: the only version check here compared `.SRCINFO` to its own
PKGBUILD, so a recipe compared to nothing could drift for a fortnight without one ✗.

**The version stamp goes in the release commit, before the tag. Only the checksums are
stamped after it** — a sha256 of a tarball GitHub does not serve until the tag exists
cannot be computed any earlier, and nothing else in the recipes has that excuse. Stamping
the version afterwards left the tagged tree naming the *previous* release, which is what
the check above now refuses; it is also the tree `release.yml`'s gate runs this script
against. COPR still builds `--commit main` rather than the tag, because the spec is read
from the commit while `Source0` is fetched from the tag tarball — the two agree once the
stamp is in the release commit.

`carrel` (AUR) and
`carrel.spec` (COPR) build from the GitHub **source** tarball, which carries `contrib/` and
`Cargo.lock`. `carrel-bin` (AUR, prebuilt) is no longer blocked: v2026.8.17 is the first
release whose archives carry the man page and completions it installs. Note that **dist
flattens `include` paths to the archive root**: `contrib/carrel.1` arrives as `carrel.1`.
AUR publishing itself still waits on Arch reopening account registration.

**`carrel-package.nix`** is the nixpkgs by-name package, ready except for its two hashes.
The hashes need nix, which `docker run --rm nixos/nix` provides — the fill-in steps are in
the file's header comment.

## `release.yml` is hand-edited, and dist must be told so

`dist-workspace.toml` carries **`allow-dirty = ["ci"]`**. Without it `dist host
--steps=create` exits 255 with "release.yml is out of date" and a diff that would revert
both edits the 2026-09-01 audit made to that file: the `gate` job, which runs the five
gates against the *tagged* commit, and every third-party action pinned to a commit SHA
rather than a mutable tag — in the one workflow holding `contents: write`. This failed the
first v2026.9.1 tag: the gates passed, then dist refused to plan.

**`dist init` is not the fix.** It "resolves" the complaint by unpinning those actions and
deleting the gate job, which is the opposite of what is wanted.

The obligation this creates lands on whoever next **bumps `cargo-dist-version`**: dist will
no longer report that file stale, so diff it by hand and port the changes, keeping the gate
job and the SHA pins. You can check the whole thing locally without burning a tag —
`dist plan` in the repo reproduces the CI failure exactly, using the pinned dist:

```bash
curl -sL -o dist.tar.xz https://github.com/axodotdev/cargo-dist/releases/download/v<PINNED>/cargo-dist-x86_64-unknown-linux-gnu.tar.xz
tar xf dist.tar.xz && ./cargo-dist-x86_64-unknown-linux-gnu/dist plan
```

If a tag does fail at `plan`, no GitHub Release is created, so the tag can simply be moved:
`git tag -d vX && git push origin :refs/tags/vX`, re-tag, re-push.

## Checksums

`updpkgsums` is the usual tool but it only runs on Arch. Anywhere else:

```bash
curl -sL -o src.tar.gz https://github.com/VaHughes/carrel/archive/v<VERSION>.tar.gz
sha256sum src.tar.gz
```

For `carrel-bin`, the per-artifact sums are published as `.sha256` files on the release.

## COPR — the command that actually works

**Live since 2026-08-16:** <https://copr.fedorainfracloud.org/coprs/vahughes/carrel/> —
Fedora 43 and 44, x86_64 and aarch64. Users install with:

```bash
sudo dnf copr enable vahughes/carrel
sudo dnf install carrel
```

Publishing a new version, from a machine with no Fedora tooling at all:

```bash
uvx --from copr-cli copr-cli whoami        # token lives in ~/.config/copr, chmod 600.
                                           # No uvx or pipx? A venv needs no root:
                                           #   python3 -m venv /tmp/copr
                                           #   /tmp/copr/bin/pip install copr-cli
uvx --from copr-cli copr-cli buildscm carrel \
  --clone-url https://github.com/VaHughes/carrel \
  --commit main \
  --spec contrib/packaging/carrel.spec \
  --type git --method rpkg \
  --enable-net on
```

Three things the earlier draft of this file got wrong, each found the hard way:

1. **`copr-cli build <project> <spec>` does not work** — `build` wants an `.src.rpm`, which
   needs `rpmbuild` locally. **`buildscm` is the one to use**: COPR clones the repo and builds
   the SRPM itself, so no Fedora tooling is needed on the dev machine.
2. **`--enable-net on` is required.** COPR builds have no network by default and `cargo build`
   must fetch the dependency tree. (The Fedora-proper answer is vendoring every crate into the
   source tarball; that is a much larger change and is not done.)
3. **Never pass `--repo` to `copr-cli create` meaning "homepage".** `--repo` adds an extra DNF
   *package* repository to every build root — pointing it at the GitHub URL made all four
   chroots fail trying to fetch repo metadata from GitHub before compiling anything. Clear it
   with `copr-cli modify carrel --repo ""`.

Also note **`--commit` must point at a revision whose spec has the right `Version:`.** Building
`v2026.8.16` produced a `2026.8.12` package, because the spec was stamped on `main` after that
tag was cut. Building `main` is correct: the spec's `Source0` still points at the *tagged*
release tarball, so the package is built from a release even though the recipe came from the
branch.

The flip side (2026-08-17): **the source is tag-frozen**, so a fix pushed to `main` after
tagging never reaches the build. If it must ship, carry it as a `Patch0` file next to the
spec (rpkg packs local sources from the spec's directory), bump `Release:`, and drop the
patch at the next release — keeping its hunks to files unchanged since the tag, dry-run
verified against the extracted tag tarball.

Verify a build without any RPM tooling by reading the repo metadata — in **all four**
chroots, because a build reported as `succeeded` can still have shipped the wrong version:

```bash
REPO=https://download.copr.fedorainfracloud.org/results/vahughes/carrel/fedora-43-x86_64
curl -sL "$REPO/repodata/repomd.xml"   # find the primary.xml.gz href, then zcat it
```

Two traps here. **`-L` is not optional**: that results path 302s to S3, and a plain `curl`
silently returns an HTML error page, so the gzip parse dies with `BadGzipFile … (b'<!')`
naming nothing about a redirect. And **the repo lists every build it has ever kept** — at
2026.9.1 that was eighteen `carrel` entries per chroot, oldest first. Take the newest, or
grep for the version you are expecting; reading the first `<package>` block reports a
release from three weeks ago and looks exactly like a failure.

## `.SRCINFO`

`SRCINFO-carrel` is a mechanical flattening of `PKGBUILD-carrel` — **if you edit the PKGBUILD
you must edit it too.** It was once hand-written, because `makepkg --printsrcinfo` needs Arch
and the maintainer's machine was not; it is, so regenerate rather than hand-edit:

```bash
mkdir -p /tmp/srcinfo && cp contrib/packaging/PKGBUILD-carrel /tmp/srcinfo/PKGBUILD
(cd /tmp/srcinfo && makepkg --printsrcinfo) > contrib/packaging/SRCINFO-carrel
```

It must be named `PKGBUILD` in that directory or makepkg will not read it. Diffing the
generated file against the committed one is the check worth having — at 2026.9.1 they came
out byte-identical, which is the result you want and not a reason to skip the step.

- **`PKGBUILD-carrel`** — AUR source package. Publish: create the AUR package via
  `ssh aur@aur.archlinux.org`, clone `ssh://aur@aur.archlinux.org/carrel.git`, copy this file
  in as `PKGBUILD`, `makepkg --printsrcinfo > .SRCINFO`, commit, push. The sum is already
  stamped by the release flow, so `updpkgsums` is only for re-deriving it.
- **`PKGBUILD-carrel-bin`** — AUR prebuilt package, same flow into `carrel-bin.git`; sums come
  from the release's `.sha256` files.
- **`carrel.spec`** — Fedora COPR. The project already exists (F43 and F44, x86_64 and
  aarch64); each release is the `buildscm` command above, **not** `copr-cli build` — see
  point 1. The same spec seeds an openSUSE OBS home project later.

The full launch order lives in RELEASING.md in the private notes repo.

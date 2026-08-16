# Packaging templates

**`carrel` (AUR) and `carrel.spec` (COPR) are stamped for v2026.8.16 and ready to publish.**
They build from the GitHub **source** tarball, which carries `contrib/` and `Cargo.lock`.

**`carrel-bin` is blocked** until a release is built with the widened `include` list in
`dist-workspace.toml`. It installs the man page and completions *from the release archive*,
and those were not in it — `include` shipped only `carrel.desktop`. Verified against the real
v2026.8.16 artifact, which would have failed the build. Note that **dist flattens `include`
paths to the archive root**: `contrib/carrel.1` arrives as `carrel.1`.

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
pipx install copr-cli                      # token lives in ~/.config/copr, chmod 600
copr-cli buildscm carrel \
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

Verify a build without any RPM tooling by reading the repo metadata:

```bash
REPO=https://download.copr.fedorainfracloud.org/results/vahughes/carrel/fedora-43-x86_64
curl -sL "$REPO/repodata/repomd.xml"   # find the filelists.xml href, then zcat it
```

## `.SRCINFO`

`SRCINFO-carrel` is a hand-written `.SRCINFO` for the source package, because
`makepkg --printsrcinfo` needs Arch. It is a mechanical flattening of the PKGBUILD — **if you
edit the PKGBUILD you must edit it too**, and on an Arch machine regenerate it properly rather
than trusting this copy.

- **`PKGBUILD-carrel`** — AUR source package. Publish: create the AUR package via
  `ssh aur@aur.archlinux.org`, clone `ssh://aur@aur.archlinux.org/carrel.git`, copy this file
  in as `PKGBUILD`, run `updpkgsums` (replaces the SKIP), `makepkg --printsrcinfo > .SRCINFO`,
  commit, push.
- **`PKGBUILD-carrel-bin`** — AUR prebuilt package, same flow into `carrel-bin.git`; sums come
  from the release's `.sha256` files.
- **`carrel.spec`** — Fedora COPR. Publish: `copr-cli create carrel --chroot fedora-42-x86_64
  --chroot fedora-42-aarch64` (plus current releases/EPEL as desired), then
  `copr-cli build carrel carrel.spec`. The same spec seeds an openSUSE OBS home project later.

The full launch order lives in RELEASING.md in the private notes repo.

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

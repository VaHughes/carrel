# Staged packaging templates

Everything in this directory references release URLs that only exist once the repo is public
and the first `v*` tag has produced a release. **Nothing here can be published before then.**
Versions and checksums are stamped at publish time (`SKIP` placeholders mark every spot).

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

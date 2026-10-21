# Packaging templates

**All five recipes are stamped for v2026.10.21**, and `scripts/check-packaging.sh` now
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

**After the tag, `.github/workflows/publish.yml` does the rest with no hand step.** It runs
when `Release` succeeds: `cargo publish` (core, then carrel) with a check of the rendered
crate page and an unlocked `cargo install`; the Homebrew formula committed to the tap; the
nix hashes computed and built with on a hosted runner; the checksum stamp committed to
`main` (`scripts/stamp-release.sh`); and the COPR build submitted from the tag and waited
on. Every job is idempotent, so a failure is retried by re-running it, or by running the
workflow from Actions with the tag.

`carrel` (AUR) and
`carrel.spec` (COPR) build from the GitHub **source** tarball, which carries `contrib/` and
`Cargo.lock`. `carrel-bin` (AUR, prebuilt) is no longer blocked: v2026.8.17 is the first
release whose archives carry the man page and completions it installs. Note that **dist
flattens `include` paths to the archive root**: `contrib/carrel.1` arrives as `carrel.1`.
AUR publishing itself still waits on Arch reopening account registration.

**`carrel-package.nix`** is the nixpkgs by-name package. `publish.yml` fills its two
hashes after each release and builds it with them, so the copy on `main` is ready to drop
into a nixpkgs checkout; until the first stamp it carries `lib.fakeHash`.

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

`publish.yml` submits each release's build from the tag and waits for it. By hand, from a
machine with no Fedora tooling at all:

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
  aarch64); each release is the `buildscm` command above (run by `publish.yml`), **not**
  `copr-cli build` — see point 1. The same spec seeds an openSUSE OBS home project later.
- **`.deb`** (Debian, Ubuntu) — built and attached by CI, no manual step; see below.

The full launch order lives in RELEASING.md in the private notes repo.

## `.deb` — Debian and Ubuntu

Every release gets `carrel_<VERSION>-1_amd64.deb` and `carrel_<VERSION>-1_arm64.deb`,
each with a `.sha256` beside it. Users install with `sudo apt install ./carrel_*.deb`.
There is no apt repository, so there are no automatic updates.

**What it is:** a repack of the static **musl** release archive, the same model as
`carrel-bin`. The binary inside is the archive's binary, byte for byte, which is why the
package has no `Depends` at all and one file installs on every supported Debian and Ubuntu.
The gnu archives would need a `libc6 (>= 2.35)` floor from the Ubuntu 22.04 release
container and would gain nothing for it.

- **Recipe:** `[package.metadata.deb]` in `crates/carrel/Cargo.toml`, read by
  [cargo-deb](https://crates.io/crates/cargo-deb) (pinned in `deb.yml`). Its asset
  sources are the flat archive names under `target/release/`. `scripts/build-deb.sh
  ARCHIVE [OUTDIR]` unpacks the archive there, in a private `CARGO_TARGET_DIR`, and runs
  `cargo deb --no-build`. **A plain `cargo deb` in the repo does not work, on purpose:**
  the package is never built from a fresh compile.
- **Paths:** `/usr/bin/carrel`, `/usr/share/man/man1/carrel.1.gz` (`gzip -9n`, done by
  the script, because cargo-deb skips compressing anything under the target directory),
  `/usr/share/bash-completion/completions/carrel`,
  `/usr/share/zsh/vendor-completions/_carrel` (Debian's zsh path, not `site-functions`),
  `/usr/share/fish/vendor_completions.d/carrel.fish`,
  `/usr/share/applications/carrel.desktop`, plus the licence texts, `THIRD-PARTY-NOTICES`
  (the embedded hyphenation data's licences) and a generated DEP-5
  `copyright` under `/usr/share/doc/carrel/`.
- **Version:** `<Cargo.toml version>-1`. CalVer sorts correctly under dpkg
  (`2026.10.8-1` > `2026.9.23-1`, `2027.1.1-1` > `2026.12.31-1`), so there is no epoch.
  Neither Debian nor Ubuntu has a package named `carrel` (checked 2026-10-19), so nothing
  is declared to conflict with. If a distro package ever appears, theirs will likely be
  `<version>-1` too: decide then, and never add an epoch, which can never be removed.
- **Q21:** no maintainer scripts at all. The desktop entry's `MimeType=` makes carrel
  *available* for markdown, and desktop-file-utils' own dpkg trigger refreshes the cache;
  nothing makes it the default. `check-packaging.sh` fails a package that has a
  maintainer script or writes under `/etc`.
- **Reproducible:** the same archive gives the same `.deb`, byte for byte.
  `SOURCE_DATE_EPOCH` defaults to the binary's mtime in the archive.

**Why its own workflow, `deb.yml`, and not a dist custom publish job.** release.yml is
hand-edited (see above), so a dist hook would be one more edit to port at every dist
upgrade. Worse, the stanza dist 0.32 generates for a custom publish job sets the job's
permissions to `id-token` and `packages` only, which leaves the called workflow without
`contents: write`, so its `gh release upload` would fail. `deb.yml` triggers on
`workflow_run` of `Release`. **`release: published` would never fire**, because
release.yml creates the release with `GITHUB_TOKEN` and GitHub starts no workflow for
events caused by that token.

`deb.yml` checks out the **tag**, downloads both musl archives, checks them against their
`.sha256`, builds, runs `check-packaging.sh` on each `.deb`, then uploads with `--clobber`.
To re-run it, or to package a release that already exists: Actions → deb → Run workflow
with the tag. That only works for a tag whose tree has `scripts/build-deb.sh`. For an
older release, build the packages locally with the script and upload them by hand.

Check a built package without dpkg (`ar` is enough) or with it:

```bash
./scripts/build-deb.sh carrel-x86_64-unknown-linux-musl.tar.xz /tmp/debs
./scripts/check-packaging.sh /tmp/debs/carrel_*_amd64.deb
```


## Ghostmachine release execution

`release.yml` routes x64 Linux builds to `ghost-ci-public`, with an Ubuntu 22.04
container preserving the GNU ABI baseline. Its container bootstrap installs the
compiler, Git, curl, OpenSSL headers and musl tools before the normal dist steps.
Native ARM and macOS artifacts still use GitHub's hosted runners. Keep all six
targets, the gate dependency and action SHA pins when regenerating the workflow.
The plan/global/host jobs also use Ghostmachine for owner events; external-fork
PRs retain hosted checks. See `docs/DEVELOPMENT.md` for the required fork approval
policy before enabling any public self-hosted runner.

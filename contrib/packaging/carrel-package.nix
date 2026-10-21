# nixpkgs by-name package for carrel — destined for
# pkgs/by-name/ca/carrel/package.nix in a nixpkgs PR.
#
# The two hashes are filled after each release by .github/workflows/publish.yml,
# which runs this procedure on a hosted runner and builds the package with them:
#   1. nix-build -E 'with import <nixpkgs> {}; callPackage ./carrel-package.nix {}'
#      with both set to lib.fakeHash — the build fails printing the real `src`
#      hash ("got: sha256-…"); put it in.
#   2. Re-run; it fails again printing the real `cargoHash`; put that in.
#   3. Re-run once more to a successful build, then `./result/bin/carrel --version`.
# Between the release commit and the stamp, they are the previous release's.
# Submitting: copy into a nixpkgs checkout as pkgs/by-name/ca/carrel/package.nix,
# add the maintainer to maintainers/maintainer-list.nix (first PR can do both),
# commit as "carrel: init at <version>", open the PR against master.
{
  lib,
  rustPlatform,
  fetchFromGitHub,
  installShellFiles,
}:

rustPlatform.buildRustPackage (finalAttrs: {
  pname = "carrel";
  version = "2026.10.8";

  src = fetchFromGitHub {
    owner = "VaHughes";
    repo = "carrel";
    tag = "v${finalAttrs.version}";
    hash = lib.fakeHash;
  };

  cargoHash = lib.fakeHash;

  nativeBuildInputs = [ installShellFiles ];

  postInstall = ''
    installManPage contrib/carrel.1
    installShellCompletion \
      --bash contrib/completions/carrel.bash \
      --zsh contrib/completions/carrel.zsh \
      --fish contrib/completions/carrel.fish
    install -Dm644 contrib/carrel.desktop $out/share/applications/carrel.desktop
    install -Dm644 THIRD-PARTY-NOTICES $out/share/doc/carrel/THIRD-PARTY-NOTICES
  '';

  meta = {
    description = "Terminal markdown reader with search that survives reflow and resize";
    homepage = "https://github.com/VaHughes/carrel";
    changelog = "https://github.com/VaHughes/carrel/blob/v${finalAttrs.version}/CHANGELOG.md";
    # Carrel's code is MIT OR Apache-2.0; the embedded hyphenation data adds
    # BSD-3-Clause, Knuth-CTAN and an all-permissive notice (SPDX
    # FSFAP-no-warranty-disclaimer, which nixpkgs has no entry for — fsfap is
    # the nearest). Cargo.toml has the exact expression; THIRD-PARTY-NOTICES
    # the texts.
    license = with lib.licenses; [
      mit
      asl20
      bsd3
      fsfap
      knuth
    ];
    mainProgram = "carrel";
    maintainers = [ ]; # add yourself: lib.maintainers.<handle>, registered in the same PR
  };
})

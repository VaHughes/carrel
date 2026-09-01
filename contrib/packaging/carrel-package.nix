# nixpkgs by-name package for carrel — destined for
# pkgs/by-name/ca/carrel/package.nix in a nixpkgs PR.
#
# The two `lib.fakeHash` values must be filled on a machine with nix:
#   1. nix-build -E 'with import <nixpkgs> {}; callPackage ./carrel-package.nix {}'
#      — the build fails printing the real `src` hash ("got: sha256-…"); paste it in.
#   2. Re-run; it fails again printing the real `cargoHash`; paste that in.
#   3. Re-run once more to a successful build, then `./result/bin/carrel --version`.
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
  version = "2026.8.31";

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
  '';

  meta = {
    description = "Terminal markdown reader with search that survives reflow and resize";
    homepage = "https://github.com/VaHughes/carrel";
    changelog = "https://github.com/VaHughes/carrel/blob/v${finalAttrs.version}/CHANGELOG.md";
    license = with lib.licenses; [
      mit
      asl20
    ];
    mainProgram = "carrel";
    maintainers = [ ]; # add yourself: lib.maintainers.<handle>, registered in the same PR
  };
})

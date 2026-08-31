# Fedora COPR spec. Build with: copr-cli build <project> carrel.spec
# rust2rpm-shaped but hand-trimmed: carrel vendors nothing and needs no C toolchain.
Name:           carrel
Version:        2026.8.31
Release:        1%{?dist}
Summary:        A quiet place to read your markdown — a terminal markdown reader
License:        MIT OR Apache-2.0
URL:            https://github.com/VaHughes/carrel
Source0:        %{url}/archive/v%{version}/carrel-%{version}.tar.gz
BuildRequires:  cargo
BuildRequires:  rust >= 1.90

%description
Carrel is a free and open-source terminal markdown reader: search that
survives reflow and terminal resize, 17 themes, card view for wide tables,
mermaid box art, wikilinks, and a home screen that lists the markdown
around you.

%prep
%autosetup -n carrel-%{version} -p1

%build
cargo build --release -p carrel

%install
install -Dm755 target/release/carrel %{buildroot}%{_bindir}/carrel
install -Dm644 contrib/carrel.desktop %{buildroot}%{_datadir}/applications/carrel.desktop
install -Dm644 contrib/carrel.1 %{buildroot}%{_mandir}/man1/carrel.1
install -Dm644 contrib/completions/carrel.bash %{buildroot}%{_datadir}/bash-completion/completions/carrel
install -Dm644 contrib/completions/carrel.zsh %{buildroot}%{_datadir}/zsh/site-functions/_carrel
install -Dm644 contrib/completions/carrel.fish %{buildroot}%{_datadir}/fish/vendor_completions.d/carrel.fish
install -Dm644 LICENSE-MIT %{buildroot}%{_datadir}/licenses/%{name}/LICENSE-MIT
install -Dm644 LICENSE-APACHE %{buildroot}%{_datadir}/licenses/%{name}/LICENSE-APACHE

%check
cargo test --workspace

%files
%{_bindir}/carrel
%{_datadir}/applications/carrel.desktop
%{_mandir}/man1/carrel.1*
%{_datadir}/bash-completion/completions/carrel
%{_datadir}/zsh/site-functions/_carrel
%{_datadir}/fish/vendor_completions.d/carrel.fish
%license LICENSE-MIT LICENSE-APACHE
%doc README.md CHANGELOG.md

%changelog
* Mon Aug 31 2026 Joshua Hughes <hughes238@gmail.com> - 2026.8.31-1
- The home screen notices files written while it is up: a new document
  appears, a deleted one leaves, an edited one moves up the list
- The directory picker opens holding the directory you are already in,
  instead of an empty input meaning the filesystem root

* Thu Aug 27 2026 Joshua Hughes <hughes238@gmail.com> - 2026.8.27-1
- The logo, the demo and three links render on the crates.io page again;
  they had resolved against the crate's subdirectory, not the repo root
- The AUR .SRCINFO no longer disagrees with its PKGBUILD about which tag
  to fetch; this spec's changelog regained two skipped releases

* Wed Aug 26 2026 Joshua Hughes <hughes238@gmail.com> - 2026.8.26-1
- <details> blocks fold like sections; % jumps between a footnote and its
  definition; l lists what a note links out to, " lists your bookmarks
- Fuzzy-matching pickers; document card (I); paragraph spotlight (S);
  read-aloud (A); navigable task lists; carrel --render styles a pipe

* Sat Aug 22 2026 Joshua Hughes <hughes238@gmail.com> - 2026.8.22-1
- Continue reading on the home screen; bookmarks; the outline in the margin
- Backlinks (L) and frontmatter titles in the file list
- Scrolling fast no longer eats characters; every frame is one synchronized
  update; the man page documents every reader key again

* Fri Aug 21 2026 Joshua Hughes <hughes238@gmail.com> - 2026.8.21-1
- Diffs read as documents; git pager support
- Follow mode for growing documents; copy a code block with y
- Attached superscripts (x^2^, H~2~O); home-screen scroll and picker fixes
- Windows declined; no Windows binaries are planned

* Thu Aug 20 2026 Joshua Hughes <hughes238@gmail.com> - 2026.8.20-1
- Pipe into carrel and it streams as the producer writes; a sticky heading
  breadcrumb; section folding with za/zM/zR and click-a-heading; and a
  Homebrew formula on every release.

* Mon Aug 17 2026 Joshua Hughes <hughes238@gmail.com> - 2026.8.17-2
- Patch the picker round-trip test to pin the painter's clipping contract;
  it failed only under COPR's deep /builddir working tree.

* Mon Aug 17 2026 Joshua Hughes <hughes238@gmail.com> - 2026.8.17-1
- A 90-column reading measure with full-bleed tables and code, clickable
  home list and directory picker, time remaining in the status bar, and
  Esc clears accepted-search highlights.

* Sun Aug 16 2026 Joshua Hughes <hughes238@gmail.com> - 2026.8.16-1
- Frontmatter as a metadata card, LaTeX math, definition lists, --version,
  man page and shell completions, and the Q16 conformance suite.

* Wed Aug 12 2026 Joshua Hughes <hughes238@gmail.com> - 2026.8.12-1
- Initial package.

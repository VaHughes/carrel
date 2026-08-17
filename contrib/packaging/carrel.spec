# Fedora COPR spec. Build with: copr-cli build <project> carrel.spec
# rust2rpm-shaped but hand-trimmed: carrel vendors nothing and needs no C toolchain.
Name:           carrel
Version:        2026.8.17
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
%autosetup -n carrel-%{version}

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
* Mon Aug 17 2026 Joshua Hughes <hughes238@gmail.com> - 2026.8.17-1
- A 90-column reading measure with full-bleed tables and code, clickable
  home list and directory picker, time remaining in the status bar, and
  Esc clears accepted-search highlights.

* Sun Aug 16 2026 Joshua Hughes <hughes238@gmail.com> - 2026.8.16-1
- Frontmatter as a metadata card, LaTeX math, definition lists, --version,
  man page and shell completions, and the Q16 conformance suite.

* Wed Aug 12 2026 Joshua Hughes <hughes238@gmail.com> - 2026.8.12-1
- Initial package.

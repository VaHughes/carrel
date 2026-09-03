#!/usr/bin/env bash
# Mechanically enforce the architectural rules that make a second frontend possible.
#
# Every project examined during research that planned a second frontend for
# "later" never got one, because the first frontend's assumptions calcified into
# the shared core. Helix's own docs record it: "The `view` layer was supposed to
# be a frontend-agnostic imperative library... Currently it's tied to the
# terminal UI." Its frontend-agnostic layer defines a style bitflag including
# terminal BLINK.
#
# This script is the guard. CI runs it. See architecture.md (private notes repo) §0.

set -uo pipefail
cd "$(dirname "$0")/.." || exit 1

CORE=crates/carrel-core
fail=0

note() { printf '  \033[31m✗\033[0m %s\n' "$1"; fail=1; }
ok()   { printf '  \033[32m✓\033[0m %s\n' "$1"; }

# Search carrel-core's source, skipping comment lines — the docs necessarily
# *name* the things the rules forbid.
scan() { grep -rnE "$1" "$CORE/src" 2>/dev/null | grep -vP '^[^:]+:[0-9]+:\s*(//|/\*|\*)'; }

echo "carrel-core discipline check"

# --- Rule 1: no UI crates, ever. ---------------------------------------------
if grep -nE '^\s*(ratatui|ratatui-core|crossterm|termion|termwiz|gtk4|gtk|webkit6|iced|egui)\b' \
     "$CORE/Cargo.toml" >/dev/null 2>&1; then
  note "carrel-core declares a UI dependency in Cargo.toml"
  grep -nE '^\s*(ratatui|crossterm|termion|termwiz|gtk4|gtk|webkit6|iced|egui)\b' "$CORE/Cargo.toml"
else
  ok "no UI dependency declared"
fi

hits=$(scan '\b(use|extern crate)\s+(ratatui|ratatui_core|crossterm|termion|termwiz|gtk4|webkit6|iced|egui)\b')
if [ -n "$hits" ]; then
  note "carrel-core imports a UI crate"; echo "$hits"
else
  ok "no UI crate imported"
fi

# --- Rule 2: no ANSI escapes. The core emits semantic scopes, never colours. --
hits=$(scan '\\x1[bB]|\\033|\\e\[')
if [ -n "$hits" ]; then
  note "carrel-core contains an ANSI escape sequence"; echo "$hits"
else
  ok "no ANSI escapes"
fi

# --- Rule 3: no width-dependent layout results in the public API. ------------
# `indent: u16` is fine — it is a property of list/quote nesting, not of layout.
# A HEIGHT or a ROW COUNT is not: `fn height() -> u16` is the exact API shape
# that ended Helix's frontend-agnostic view layer.
hits=$(scan 'pub\s+fn\s+(height|row_count|rows|total_rows|line_count|scroll_row)\s*\(')
if [ -n "$hits" ]; then
  note "carrel-core exposes a width-dependent layout quantity"; echo "$hits"
else
  ok "no width-dependent layout quantity in the public API"
fi

# --- Rule 4: positions are byte offsets. There is no char_to_byte. -----------
hits=$(scan 'char_to_byte|byte_to_char|char_indices\(\)\s*\.\s*nth')
if [ -n "$hits" ]; then
  note "carrel-core converts between char and byte indices (see architecture.md §1.3)"
  echo "$hits"
else
  ok "positions are byte offsets throughout"
fi

# --- Rule 6: the TUI's state layer stays frontend-agnostic. -----------------
# action/app/layout/view are the modules a GTK frontend reuses verbatim. If
# ratatui reaches them, the second frontend is dead and nobody notices for a
# year — which is exactly what Helix's architecture doc records happening to
# its "frontend-agnostic" view layer.
echo
echo "carrel state-layer discipline check"
PURE="crates/carrel/src/action.rs crates/carrel/src/app.rs crates/carrel/src/plain.rs \
      crates/carrel/src/layout.rs crates/carrel/src/view.rs \
      crates/carrel/src/config.rs crates/carrel/src/scan.rs \
      crates/carrel/src/home.rs crates/carrel/src/images.rs \
      crates/carrel/src/state.rs crates/carrel/src/wiki.rs \
      crates/carrel/src/grep.rs crates/carrel/src/diagrams.rs \
      crates/carrel/src/footer.rs crates/carrel/src/breadcrumb.rs \
      crates/carrel/src/menu.rs"
hits=$(grep -nE '^\s*use\s+ratatui' $PURE 2>/dev/null)
if [ -n "$hits" ]; then
  note "carrel's state layer imports ratatui"; echo "$hits"
else
  ok "the state layer (action/app/layout/view/config/scan/home) is ratatui-free"
fi

echo
if [ "$fail" -ne 0 ]; then
  echo "DISCIPLINE CHECK FAILED — see architecture.md (private notes repo) §0 and §8"
  exit 1
fi
echo "All discipline checks passed."

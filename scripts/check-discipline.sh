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

# Rule 1, and rule 7's half of it, asked of cargo rather than grepped from a
# manifest: `[dependencies.ratatui]`, a renamed `tui = { package = "ratatui" }`
# and a UI crate pulled in by another dependency all show up here, under their
# real package names. A cargo that cannot answer is a failure, never a pass
# on an empty list.
ui_tree() { # DIR CRATE
  local tree hits
  tree=$(cargo tree -p "$2" -e normal,build --prefix none --format '{p}' --offline 2>/dev/null \
         || cargo tree -p "$2" -e normal,build --prefix none --format '{p}' --locked 2>&1)
  if ! printf '%s\n' "$tree" | grep -q "^$2 "; then
    note "could not list $2's dependencies (cargo tree):"; printf '%s\n' "$tree" | tail -3
  elif hits=$(printf '%s\n' "$tree" | grep -v "^$2 " | grep -E '^(ratatui|ratatui-core|ratatui-image|crossterm|termion|termwiz|gtk4|gtk|webkit6|iced|egui|winit|carrel|carrel-state) '); then
    note "$2 depends, directly or not, on a UI crate or a frontend"; echo "$hits" | sort -u
  else
    ok "no UI or frontend crate anywhere in its dependency tree"
  fi
}

echo "carrel-core discipline check"

# --- Rule 1: no UI crates, ever. ---------------------------------------------
ui_tree "$CORE" carrel-core

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

# --- Rule 6: the TUI's terminal-free modules stay terminal-free. ----------
# These are the modules the state machine is built from: testable with no
# terminal, and what a second frontend can read for how carrel behaves. If a
# terminal library reaches them, nobody notices for a year — which is exactly
# what Helix's architecture doc records happening to its "frontend-agnostic"
# view layer.
#
# Until 2026-10-31 this rule was one grep for `^\s*use\s+ratatui` over this
# list, and it passed while `app`, `footer` and `menu` reached crossterm
# through `crate::keys` (research.md Q29): a grep sees direct imports only.
# So it is two checks now, and together they cover the closure:
#   6a. no file on the list names a terminal crate on any non-comment line,
#       in a `use` or inline (`ratatui::crossterm::event::KeyCode`);
#   6b. every module a listed file reaches through `crate::` is itself on
#       the list — so nothing on it can reach a terminal crate second-hand.
echo
echo "carrel terminal-free module check"
SRC=crates/carrel/src
PURE="action annotation_state ansi app breadcrumb cli diagrams footer \
      home images keymap layout math_art menu peek plain status view"
# Modules this crate re-exports from carrel-state (`pub use carrel_state::{…}`
# in lib.rs): `crate::state` is one of those, and the compiler — not this
# script — keeps it terminal-free, because carrel-state cannot depend on a
# terminal crate (rule 7). An edge to one of them is an edge into the list.
SHARED=$(perl -0777 -ne 'print "$1\n" while /pub use carrel_state::\{([^}]*)\}/g' "$SRC/lib.rs" | tr ',' ' ')
TERM_CRATES='ratatui|ratatui_core|ratatui_image|crossterm|termion|termwiz|signal_hook'
deps() {
  # The top-level modules a file reaches through `crate::`, comments
  # stripped, `use crate::{a, b::{c, d}}` groups included.
  perl -e '
    local $/; my $s = <STDIN>;
    $s =~ s{//[^\n]*}{}g; $s =~ s{/\*.*?\*/}{}gs;
    my %m;
    # `super::x` in a top-level module is `crate::x`. Inside a nested module
    # (`mod tests`) it names the file module itself, whose item names the
    # caller filters out: only a name that is a module file counts.
    $m{"super:$1"} = 1 while $s =~ /\bsuper::([a-z_][a-z0-9_]*)/g;
    while ($s =~ /\bcrate::(\{|[a-z_][a-z0-9_]*)/g) {
      if ($1 ne "{") { $m{$1} = 1; next }
      my ($d, $i, $tok, @top) = (1, pos($s), "");
      while ($d > 0 && $i < length $s) {
        my $c = substr($s, $i++, 1);
        if ($c eq "{") { push @top, $tok if $d == 1; $tok = ""; $d++; next }
        if ($c eq "}") { push @top, $tok if $d == 1; $tok = ""; $d--; next }
        if ($c eq "," && $d == 1) { push @top, $tok; $tok = ""; next }
        $tok .= $c if $d == 1;
      }
      for (@top) { $m{$1} = 1 if /^\s*([a-z_][a-z0-9_]*)/ }
    }
    print "$_\n" for sort keys %m;' < "$1"
}
leak=""; reach=""
for m in $PURE; do
  f="$SRC/$m.rs"
  [ -f "$f" ] || { note "rule 6 lists $m, which does not exist — update the list"; continue; }
  h=$(grep -nE "\b($TERM_CRATES)::|\buse\s+($TERM_CRATES)\b" "$f" \
      | grep -vP '^[0-9]+:\s*(//|/\*|\*)' | sed "s|^|$f:|")
  [ -n "$h" ] && leak="$leak$h"$'\n'
  for d in $(deps "$f"); do
    case $d in
      super:*) d=${d#super:}; [ -f "$SRC/$d.rs" ] || [ -d "$SRC/$d" ] \
                 || case " $(echo $SHARED) " in *" $d "*) ;; *) continue ;; esac ;;
    esac
    case " $(echo $PURE $SHARED) " in *" $d "*) ;; *) reach="$reach  $m -> crate::$d"$'\n' ;; esac
  done
done
if [ -n "$leak" ]; then
  note "a terminal-free module names a terminal crate"; printf '%s' "$leak"
else
  ok "no terminal crate named in the terminal-free modules"
fi
if [ -n "$reach" ]; then
  note "a terminal-free module reaches a module that is not (list it, or cut the edge)"
  printf '%s' "$reach"
else
  ok "the terminal-free modules reach only each other"
fi

# --- Rule 7: carrel-state is shared, so it is UI-free and geometry-free. ---
# The crate every frontend links (Q29: in-process core + an app per toolkit).
# The compiler holds its dependencies, given this Cargo.toml; the script
# holds the Cargo.toml, and the SHAPES no compiler sees — Helix's shared
# crate had no terminal dependency and still defined `Rect { width: u16 }`.
echo
echo "carrel-state discipline check"
STATE=crates/carrel-state
ui_tree "$STATE" carrel-state
sscan() { grep -rnE "$1" "$STATE/src" 2>/dev/null | grep -vP '^[^:]+:[0-9]+:\s*(//|/\*|\*)'; }
hits=$(sscan '\\x1[bB]|\\033|\\e\[')
if [ -n "$hits" ]; then
  note "carrel-state contains an ANSI escape sequence"; echo "$hits"
else
  ok "no ANSI escapes"
fi
# A terminal cell is a pair of u16s, a rectangle, or a u16 named for a cell.
# One exemption, and it is marked where it is: a config key that belongs to
# the terminal (config.rs, "Whose key is it") is stored in the shared file,
# in the terminal's unit. Such an item is preceded by the line
# `// discipline: terminal key`, and nothing else is exempt.
marked() { # FILE:LINE:… -> succeed when the line above carries the marker
  local f=${1%%:*} rest=${1#*:}; local n=${rest%%:*}
  [ "$n" -gt 1 ] && sed -n "$((n - 1))p" "$f" | grep -qE '^\s*// discipline: terminal key$'
}
hits=$(sscan '\(\s*u16\s*,\s*u16\s*\)|\bRect\b|\b(col|cols|row|rows|x|y|width|height|w|h)\s*:\s*(Option<)?u16\b' \
       | while IFS= read -r h; do marked "$h" || printf '%s\n' "$h"; done)
if [ -n "$hits" ]; then
  note "carrel-state carries terminal geometry"; echo "$hits"
else
  ok "no cells, rows or rectangles"
fi
# The intents a GUI inherits carry no u16 of any kind: every number in them
# is a doc byte, a list index, or a signed step.
hits=$(grep -nE '\bu16\b' "$STATE/src/intent.rs" | grep -vP '^[0-9]+:\s*(//|/\*|\*)')
if [ -n "$hits" ]; then
  note "an intent carries a u16 — a cell, not a document position"; echo "$hits"
else
  ok "intents are addressed by the document"
fi

echo
if [ "$fail" -ne 0 ]; then
  echo "DISCIPLINE CHECK FAILED — see architecture.md (private notes repo) §0 and §8"
  exit 1
fi
echo "All discipline checks passed."

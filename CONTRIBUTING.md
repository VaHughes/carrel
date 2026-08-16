# Contributing to Carrel

Thank you for wanting to. Carrel is small enough to understand in an afternoon, and it intends to
stay that way.

## The one rule that outranks everything

> There is exactly one authoritative coordinate space: a byte offset into a flattened, unwrapped
> display text (`Document::text`). Screen row, wrap column, and highlight rectangle are *derived*
> from `(document, width)` — recomputed on resize, never stored.

Every design decision follows from that sentence. The mechanical consequences are what
`./scripts/check-discipline.sh` enforces: no UI dependency in the core, no ANSI or RGBA leaving
it, no width-dependent quantity in its public API, positions as byte offsets everywhere.

Contributions are welcome **inside** the invariant. A change that stores a display coordinate,
adds a UI dependency to the core, or makes search state width-dependent will be declined however
good it looks, and `./scripts/check-discipline.sh` — which CI runs — will usually decline it
before a human has to.

## The gates

CI runs exactly what local development runs. All five must be green:

```bash
cargo test --workspace                             # every test
cargo clippy --workspace --all-targets             # ZERO warnings is the bar
cargo fmt --all --check
./scripts/check-discipline.sh                      # the architectural guard
./scripts/check-packaging.sh                       # recipes name files that ship
```

## How changes happen here

- **Tests first.** Every bug fix starts with a test that fails against the current code. If you
  did not watch it fail, you do not know what it tests.
- **Measure before optimizing.** Several of this repo's design decisions were settled by a
  benchmark overruling the spec (`cargo bench -p carrel-core --bench wrap`). Claims about
  performance come with numbers or they are opinions.
- **Decided questions stay decided** unless new evidence arrives — the parser (pulldown-cmark),
  the text storage (`String`, no rope), the highlighter (syntect), and the one-coordinate-space
  rule were each researched at length. Open an issue with the new evidence before a PR that
  re-litigates one.
- **Specs and changelogs are part of the change.** Behaviour changes update `CHANGELOG.md`;
  design-level changes get a short design doc reviewed in the PR.

## Scope guidance

Good first contributions: terminal quirks on hardware the maintainer does not have (image
protocols, wide-character edge cases), packaging, additional syntax-highlighting classification
prefixes, documentation that turned out to be wrong.

Talk first (open an issue) before: anything in `layout/`, anything touching positions, the GTK
frontend, or a new dependency — this project counts binary bytes and every dependency in the tree
is there on purpose.

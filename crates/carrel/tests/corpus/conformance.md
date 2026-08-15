---
title: Conformance
tags: markdown, carrel
nested:
  - one
  - two
---

# Conformance corpus

This document contains every construct carrel is asked about, including the
ones it deliberately does not support. `tests/conformance.rs` asserts what
happens to each.

## CommonMark blocks

A paragraph with *emphasis*, **strong**, `code`, and a [link](https://example.test).

> A block quote.
>
> > Nested.

- bullet one
- bullet two
  - nested bullet

1. ordered one
2. ordered two

```rust
fn code_block() -> u8 { 7 }
```

    an indented code block

---

Setext heading
==============

## GFM

| column | other |
|--------|-------|
| a      | b     |

~~strikethrough~~

- [x] a done task
- [ ] a pending task

A footnote reference.[^1]

[^1]: The footnote definition.

> [!NOTE]
> A GFM alert.

Autolink in angle brackets: <https://example.test>

Bare extended autolink: www.example.com, with trailing punctuation.

## Extensions carrel supports

Term
: The definition of the term.

Word boundary scripts: x ^2^ and log ~2~ n.

Inline math: $E = mc^2$ and $\alpha \ge 0$.

$$
\frac{a+b}{c}
$$

$$\sqrt{x^2+y^2}$$

A wikilink: [[other-note]]

## Deliberately NOT supported

Each renders as literal text, and that is the documented behaviour — see
the "will not support" table in the design spec.

Highlight: ==highlight==

Emoji shortcode: :smile:

Directive block:

::: note
A directive.
:::

Attached scripts, an upstream pulldown-cmark limitation: x^2^ and H~2~O.

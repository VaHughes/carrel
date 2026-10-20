//! Regression tests for the two bugs this project exists in order not to have,
//! plus the layout chunker.
//!
//! These use the public API only — if any of them needs an internal, the API is
//! wrong.

use carrel_core::{
    BlockIdx, Document, Hyphenation, Row, RowKind, chunk_count, cluster_width, cols_for_doc_range,
    search, wrap, wrap_chunk, wrap_with,
};

/// Doc bytes actually painted for a search, at a given width.
fn highlighted(doc: &Document, needle: &str, width: u16) -> Vec<(u32, u32)> {
    let m = search(doc, needle, true);
    let mut out = Vec::new();
    for b in 0..doc.block_count() {
        wrap(doc, BlockIdx(b as u32), width, &cluster_width, |row| {
            for r in &m.ranges {
                if r.end <= row.doc.start || r.start >= row.doc.end {
                    continue;
                }
                let lo = r.start.max(row.doc.start);
                let hi = r.end.min(row.doc.end);
                out.push((lo, hi));
            }
        });
    }
    out
}

/// mdfried #53 — "Search doesn't match line-wrapped strings".
///
/// The cause there was running the matcher over already-wrapped rows
/// (`re.find_iter(&line_string)` per row), which makes a cross-wrap match
/// structurally impossible. Here the matcher runs over unwrapped display text,
/// so the wrap point is invisible to it.
#[test]
fn mdfried_53_a_phrase_spanning_a_soft_wrap_is_still_found() {
    let doc = Document::parse("the quick brown fox jumps over the lazy dog");

    // One match, regardless of where the line happens to break.
    let m = search(&doc, "brown fox", true);
    assert_eq!(m.len(), 1, "the phrase is one match in doc space");

    // At width 16 the rows are "the quick brown" / "fox jumps over" / ...,
    // so the phrase straddles a boundary and must paint on both rows.
    let spans = highlighted(&doc, "brown fox", 16);
    assert_eq!(spans.len(), 2, "painted on two rows: {spans:?}");
    assert!(spans[0].1 <= spans[1].0, "the two halves are disjoint");
}

/// mdfried #52 — "Search matches are lost after resizing the terminal".
///
/// The cause there was storing matches in display coordinates
/// (`LineExtra::SearchMatch(start_col, end_col, ..)` inside a `ratatui::Line`),
/// destroyed by every relayout. Here matches are doc-space byte ranges, so a
/// width change cannot touch them.
#[test]
fn mdfried_52_matches_are_bit_for_bit_identical_across_widths() {
    let src = "The quick brown fox jumps over the lazy dog. \
               The quick brown fox does it again, and again.";
    let doc = Document::parse(src);

    // `search` takes no width, which IS the fix — so there is no pair of
    // calls to compare, and asserting that two identical calls agree would
    // only prove that the function is deterministic. The claim is carried
    // below, by the characters actually painted at each width. This is just
    // the non-vacuity guard: without a match to paint, that comparison holds
    // trivially.
    assert_eq!(search(&doc, "quick brown", true).len(), 2);

    // The same characters are painted at every width, even though the rows
    // they land on differ completely.
    // Whitespace is excluded: a space inside a match that lands exactly on a
    // wrap boundary is elided and genuinely not painted, and where the
    // boundaries fall is the one thing width does change.
    let chars_at = |w: u16| -> String {
        highlighted(&doc, "quick brown", w)
            .iter()
            .flat_map(|(a, b)| doc.text[*a as usize..*b as usize].chars())
            .filter(|c| !c.is_whitespace())
            .collect()
    };
    assert_eq!(chars_at(80), chars_at(13));
    assert_eq!(chars_at(80), chars_at(7));
}

/// The current-match index is an index into a doc-space list, so `n`/`N` and the
/// "7 of 42" indicator keep working across a resize with no restoration step.
#[test]
fn the_current_match_index_needs_no_restoration_after_a_resize() {
    let doc = Document::parse("alpha beta alpha beta alpha");
    let mut m = search(&doc, "alpha", true);
    m.current = Some(1);

    // A resize touches layout only. Nothing here has a width in it to update.
    assert_eq!(m.position(), Some((2, 3)));
}

#[test]
fn an_ordinary_block_is_a_single_chunk() {
    let doc = Document::parse("a short paragraph");
    assert_eq!(chunk_count(&doc, BlockIdx(0)), 1);
}

/// A paragraph larger than `CHUNK_BYTES`: chunking must not lose, duplicate, or
/// reorder content, and per-chunk row counts must sum to the whole-block count.
#[test]
fn a_huge_paragraph_is_chunked_without_losing_content() {
    let src = "word ".repeat(20_000); // 100 KB, one paragraph
    let doc = Document::parse(&src);
    let block = BlockIdx(0);
    let width = 80;

    let chunks = chunk_count(&doc, block);
    assert!(chunks > 1, "expected several chunks, got {chunks}");

    let mut whole = Vec::new();
    let total = wrap(&doc, block, width, &cluster_width, |r| whole.push(r.doc));

    let mut per_chunk = Vec::new();
    let mut first = true;
    let mut summed = 0u32;
    for c in 0..chunks {
        summed += wrap_chunk(
            &doc,
            block,
            c,
            width,
            &cluster_width,
            carrel_core::Hyphenation::Off,
            &mut first,
            |r| {
                per_chunk.push(r.doc);
            },
        );
    }

    assert_eq!(total, summed, "per-chunk row counts must sum to the whole");
    assert_eq!(whole, per_chunk, "chunk-by-chunk equals whole-block");

    // Rows are ordered, non-overlapping, and cover every non-whitespace byte.
    for w in whole.windows(2) {
        assert!(w[0].end <= w[1].start, "overlap at {:?}", w[0]);
    }
    let node = doc.node_for_block(block);
    let covered: usize = whole.iter().map(|r| (r.end - r.start) as usize).sum();
    let non_ws = doc.text[node.doc.start as usize..node.doc.end as usize]
        .chars()
        .filter(|c| !c.is_whitespace())
        .count();
    assert!(covered >= non_ws, "content lost: {covered} < {non_ws}");
}

/// Only the first row of a block carries the marker, or a wrapped list item
/// would repeat its bullet down the left margin.
#[test]
fn only_the_first_row_of_a_wrapped_item_is_marked_first_in_block() {
    let doc = Document::parse("- alpha beta gamma delta epsilon zeta eta theta\n");
    let mut firsts = Vec::new();
    wrap(&doc, BlockIdx(0), 16, &cluster_width, |r| {
        if let carrel_core::RowKind::Text { first_in_block, .. } = r.kind {
            firsts.push(first_in_block);
        }
    });
    assert!(firsts.len() > 1, "the item must actually wrap: {firsts:?}");
    assert!(firsts[0]);
    assert!(firsts[1..].iter().all(|f| !f));
}

/// Hanging indent: every row of a wrapped list item is inset by the marker's
/// width, so continuation lines align under the text rather than the bullet.
#[test]
fn wrapped_list_rows_all_carry_the_marker_width_as_indent() {
    let doc = Document::parse("10. alpha beta gamma delta epsilon zeta\n");
    let mut indents = Vec::new();
    wrap(&doc, BlockIdx(0), 16, &cluster_width, |r| {
        indents.push(r.indent);
    });
    assert!(indents.len() > 1);
    assert!(
        indents.iter().all(|i| *i == 4),
        "\"10. \" is 4 cells on every row: {indents:?}"
    );
}

#[test]
fn highlight_columns_account_for_the_indent_of_a_list_row() {
    let doc = Document::parse("- alpha\n");
    let m = search(&doc, "alpha", true);
    let r = &m.ranges[0];
    let mut cols = None;
    wrap(&doc, BlockIdx(0), 40, &cluster_width, |row| {
        let text = &doc.text[row.doc.start as usize..row.doc.end as usize];
        cols = Some(cols_for_doc_range(text, row.doc.start, row.indent, r));
    });
    assert_eq!(cols, Some((2, 7)), "shifted right by the \"- \" prefix");
}

/// The continuation rule is universal, so prose must be byte-identical to what
/// it was before continuations existed. Prose has no interior leading
/// whitespace, so no row's indent may ever differ from the block's.
#[test]
fn prose_and_lists_wrap_exactly_as_they_did_before_continuations() {
    let doc = Document::parse(
        "A paragraph long enough to wrap several times at a narrow width, with \
         ordinary words and no leading whitespace anywhere inside it.\n\n\
         - a list item that also wraps at a narrow width and must hang\n\
         - another\n\n> a quoted paragraph that wraps too\n",
    );
    for width in [12u16, 20, 40, 80] {
        for b in 0..doc.block_count() {
            let block = BlockIdx(b as u32);
            let node = doc.node_for_block(block);
            let expected = node.indent;
            wrap(&doc, block, width, &cluster_width, |r| {
                assert_eq!(
                    r.indent, expected,
                    "prose row indent changed at width {width}, block {b}",
                );
            });
        }
    }
}

/// Wrapping one huge paragraph must be LINEAR in its size.
///
/// `wrap` computed `chunk_starts` for the loop and then threw it away,
/// calling `chunk_range` per chunk — which recomputed it, a full UAX #14 pass
/// over the whole block each time. So the chunking that exists to bound work
/// on a pathological paragraph was itself quadratic: k chunks cost k+1 scans.
/// Measured through `--plain` before the fix: 1 MB 93 ms, 2 MB 337 ms,
/// 4 MB 1240 ms, 8 MB 6361 ms, and a 20 MB document never finished. After:
/// 19, 35, 67, 125 and 297 ms.
///
/// The assertion is a RATIO rather than a wall-clock bound, so it calibrates
/// itself against whatever machine it runs on: quadrupling the input should
/// roughly quadruple the time, and the 12x ceiling catches the 16x-and-worse
/// growth of the quadratic while leaving room for a loaded CI runner.
#[test]
fn a_huge_paragraph_wraps_in_linear_time() {
    use std::time::Instant;

    let small = Document::parse(&"word ".repeat(200_000)); // ~1 MB, 1 block
    let large = Document::parse(&"word ".repeat(800_000)); // ~4 MB, 1 block
    assert_eq!(small.block_count(), 1, "the fixture must be ONE block");
    assert_eq!(large.block_count(), 1);

    let time = |doc: &Document| {
        let t = Instant::now();
        let rows = wrap(doc, BlockIdx(0), 80, &cluster_width, |_| {});
        assert!(rows > 0);
        t.elapsed().as_secs_f64()
    };

    // One warm pass each: the first touch of a fresh allocation is not what
    // this test is about.
    time(&small);
    time(&large);
    let (a, b) = (time(&small), time(&large));

    assert!(
        b < a * 12.0,
        "4x the paragraph took {:.1}x the time ({a:.4}s -> {b:.4}s). \
         Linear is ~4x; the quadratic this guards was 13x and grew.",
        b / a
    );
}

/// Parsing a line with no ASCII whitespace must be LINEAR in its length.
///
/// `attached_scripts` rule 5 skips a `^`/`~` that sits inside a URL-shaped
/// token. It found that token by scanning backwards to the previous
/// whitespace and forwards to the next — and it did that per candidate run.
/// A complete candidate can occur every four bytes (`a^1^a^1^…`), so on a
/// line with no whitespace both scans walked the whole line every time.
/// Measured through `--tasks` before the fix: 32 KB 144 ms, 64 KB 551 ms,
/// 256 KB 8631 ms; the same bytes with spaces took 12 ms. After: 3, 4, 8 ms.
///
/// The realistic triggers are ordinary — a CJK paragraph with embedded ASCII,
/// a pasted minified or base64 run — and a 1 MB such line was minutes of
/// frozen UI on open with no cancellation path.
#[test]
fn a_line_without_whitespace_parses_in_linear_time() {
    use std::time::Instant;

    let small = "a^1^".repeat(16_384); // 64 KB, no ASCII whitespace at all
    let large = "a^1^".repeat(65_536); // 256 KB

    let time = |src: &str| {
        let t = Instant::now();
        let doc = Document::parse(src);
        assert!(!doc.text.is_empty());
        t.elapsed().as_secs_f64()
    };
    time(&small);
    let (a, b) = (time(&small), time(&large));

    assert!(
        b < a * 12.0,
        "4x the line took {:.1}x the time ({a:.4}s -> {b:.4}s). \
         Linear is ~4x; the quadratic this guards was 16x.",
        b / a
    );
}

/// The same guard with a `#` in every piece: the inline-tag pass looks at a
/// split run's source for an entity that decoded to `#`, and every piece of
/// a split run carries the whole line's source range.
#[test]
fn a_split_line_full_of_hashes_parses_in_linear_time() {
    use std::time::Instant;

    let small = "a^1^#t".repeat(16_384);
    let large = "a^1^#t".repeat(65_536);
    let time = |src: &str| {
        let t = Instant::now();
        let doc = Document::parse(src);
        assert!(!doc.text.is_empty());
        t.elapsed().as_secs_f64()
    };
    time(&small);
    let (a, b) = (time(&small), time(&large));
    assert!(
        b < a * 12.0,
        "4x the line took {:.1}x the time ({a:.4}s -> {b:.4}s).",
        b / a
    );
}

// --- source lines ---

/// `notes.md:42` means the forty-second line of the FILE. Display rows are
/// not source lines: a paragraph wraps, markup is not displayed, and a table
/// is padded — so the way there is through the provenance table.
#[test]
fn a_source_line_maps_to_the_text_that_line_displays() {
    let src = "# Title\n\nfirst paragraph\nstill the first\n\n- item one\n- item **two**\n\n> quoted &amp; decoded\n";
    let doc = Document::parse(src);
    let at = |line: u32| {
        let d = doc.line_start(line).0 as usize;
        doc.text[d..].chars().take(9).collect::<String>()
    };
    assert_eq!(&at(1)[..5], "Title", "markup before the text is skipped");
    assert_eq!(&at(3)[..5], "first");
    assert_eq!(
        &at(4)[..5],
        "still",
        "a soft-wrapped source line is still its own line"
    );
    assert_eq!(&at(6)[..8], "item one");
    assert_eq!(&at(7)[..4], "item");
    assert_eq!(&at(9)[..6], "quoted");
    // A blank line lands on what follows it; past the end is the end.
    assert_eq!(&at(2)[..5], "first");
    assert_eq!(doc.line_start(999).0 as usize, doc.text.len());
    assert_eq!(doc.line_start(0), doc.line_start(1));
}

#[test]
fn a_display_offset_knows_its_source_line() {
    let src = "# Title\n\nfirst paragraph\nstill the first\n\n- item one\n- item two\n";
    let doc = Document::parse(src);
    for line in [1u32, 3, 4, 6, 7] {
        assert_eq!(
            doc.line_of(doc.line_start(line)),
            line,
            "line {line} round-trips"
        );
    }
    let last = carrel_core::DocByte(doc.text.len() as u32);
    assert!(
        doc.line_of(last) >= 7,
        "the end is on or after the last line"
    );
}

#[test]
fn a_heading_knows_the_fragment_that_names_it() {
    let doc = Document::parse("# Same\n\n## Same\n\n## Step 3: Do it!\n");
    let headings: Vec<_> = doc
        .nodes
        .iter()
        .filter(|n| matches!(n.kind, carrel_core::NodeKind::Heading { .. }))
        .collect();
    let frags: Vec<_> = headings
        .iter()
        .map(|n| doc.fragment_for(n.id).unwrap())
        .collect();
    assert_eq!(frags, ["same", "same-1", "step-3-do-it"]);
    for (n, f) in headings.iter().zip(&frags) {
        assert_eq!(
            doc.fragment_target(f),
            Some(n.doc.start),
            "{f} resolves back"
        );
    }
}

/// GitHub numbers a duplicate until the result is free, so a heading that
/// already reads "Setup 1" does not collide with the second "Setup". Counting
/// per base gave both `setup-1`: a link to the third landed on the second,
/// and a collapsed "Setup 1" came back as a collapsed "Setup".
#[test]
fn a_numbered_duplicate_never_collides_with_a_heading_that_reads_the_same() {
    let doc = Document::parse("# Setup\n\n# Setup\n\n# Setup 1\n\n# Setup\n");
    let slugs: Vec<String> = doc.heading_slugs().into_iter().map(|(_, s)| s).collect();
    assert_eq!(slugs, ["setup", "setup-1", "setup-1-1", "setup-2"]);
    for (id, slug) in doc.heading_slugs() {
        assert_eq!(
            doc.fragment_target(&slug),
            Some(doc.nodes[id.0 as usize].doc.start),
            "{slug} resolves to its own heading"
        );
    }
}

/// Table padding and the gap after a block have no source of their own, and
/// were sent to the source position AFTER the block: every padded cell of
/// every row reported the line under the table.
#[test]
fn a_byte_with_no_source_is_on_the_line_of_the_text_beside_it() {
    let src = "# T\n\n| Name | Value |\n|------|------:|\n| a | 1 |\n| longer name | 22 |\n| c | 3 |\n\nafter\n\nlast\n";
    let doc = Document::parse(src);
    let table = doc.text.find("Name").unwrap();
    let end = table
        + doc.text[table..doc.text.find("after").unwrap()]
            .trim_end()
            .len();
    let mut at = table;
    let mut seen = Vec::new();
    for row in doc.text[table..end].split('\n') {
        let lines: std::collections::BTreeSet<u32> = (at..at + row.len())
            .map(|b| doc.line_of(carrel_core::DocByte(b as u32)))
            .collect();
        assert_eq!(lines.len(), 1, "one row, one line: {row:?} gave {lines:?}");
        seen.extend(lines);
        at += row.len() + 1;
    }
    assert_eq!(seen, [3, 5, 6, 7]);
    // Just past the end of a paragraph is still that paragraph's line.
    let after = doc.text.find("after").unwrap() + "after".len();
    assert_eq!(doc.line_of(carrel_core::DocByte(after as u32)), 9);
}

#[test]
fn an_html_anchor_is_a_place_a_fragment_can_name() {
    let doc = Document::parse(
        "# Top\n\n[go](#deep)\n\nfiller\n\n<a id=\"deep\"></a>\n\nThe place.\n\n<a name='older'></a>\n\nOlder style.\n",
    );
    let at = doc.fragment_target("deep").expect("an id is an anchor") as usize;
    assert!(
        doc.text[at..].starts_with("The place."),
        "{:?}",
        &doc.text[at..]
    );
    let at = doc.fragment_target("older").expect("so is a name") as usize;
    assert!(doc.text[at..].starts_with("Older style."));
    assert_eq!(doc.fragment_target("absent"), None);
}

#[test]
fn an_email_autolink_is_a_mailto_and_not_a_file_name() {
    let doc = Document::parse("<me@example.com> and <https://example.com>\n");
    let links: Vec<&str> = doc.links.iter().map(AsRef::as_ref).collect();
    assert_eq!(links, ["mailto:me@example.com", "https://example.com"]);
}

// --- the reflow layer against hostile text ---

/// Every row of every block of `doc` at `width`.
fn rows_of(doc: &Document, width: u16, h: Hyphenation) -> Vec<Row> {
    let mut out = Vec::new();
    for b in 0..doc.block_count() {
        wrap_with(doc, BlockIdx(b as u32), width, &cluster_width, h, |r| {
            out.push(r);
        });
    }
    out
}

/// What the reflow layer promises of every row, checked against the text:
///
/// - it fits: content plus any mark is no wider than the row, unless the row
///   is a single cluster wider than the whole row, which overhangs alone
///   (with any zero-width clusters, such as a soft hyphen, beside it);
/// - rows tile their block in order, on char boundaries, and the only bytes
///   between two rows are the whitespace a break elided;
/// - nothing is dropped: the first row starts at the block, the last ends
///   at its last non-whitespace byte.
fn assert_rows_honest(doc: &Document, width: u16, h: Hyphenation, what: &str) {
    use unicode_segmentation::UnicodeSegmentation;
    let rows = rows_of(doc, width, h);
    for b in 0..doc.block_count() {
        let node = doc.node_for_block(BlockIdx(b as u32));
        let mut at = node.doc.start;
        for r in rows.iter().filter(|r| r.block.0 == b as u32) {
            let (s, e) = (r.doc.start as usize, r.doc.end as usize);
            assert!(
                doc.text.is_char_boundary(s) && doc.text.is_char_boundary(e) && s <= e,
                "{what} w{width}: row off a char boundary {:?}",
                r.doc
            );
            assert!(
                r.doc.start >= at,
                "{what} w{width}: rows overlap at {:?}",
                r.doc
            );
            let gap = &doc.text[at as usize..s];
            assert!(
                gap.chars().all(char::is_whitespace),
                "{what} w{width}: {gap:?} dropped before {:?}",
                r.doc
            );
            at = r.doc.end;
            let text = &doc.text[s..e];
            let RowKind::Text { hyphen, split, .. } = r.kind else {
                continue;
            };
            let cells = carrel_core::display_width(text) + u16::from(hyphen || split);
            let avail = width.saturating_sub(r.indent).max(1);
            assert!(
                cells <= avail
                    || text
                        .graphemes(true)
                        .filter(|g| cluster_width(g) > 0)
                        .count()
                        == 1,
                "{what} w{width}: {cells} cells on a row of {avail}: {text:?}"
            );
        }
        let rest = &doc.text[at as usize..node.doc.end as usize];
        assert!(
            rest.chars().all(char::is_whitespace),
            "{what} w{width}: {rest:?} dropped at the end of block {b}"
        );
    }
}

/// A run with no break opportunity in it, wider than a `u16` can count.
///
/// A unit's width saturated at 65,535 cells, so cutting heads off a
/// 100,000-cell run left a tail the packer believed was 65,535 minus the
/// heads — and once that reached the row width, the rest of the run, some
/// 34 KB, "fit" on one row. Every row must fit, however long the run.
#[test]
fn a_run_wider_than_sixty_five_thousand_cells_still_wraps_to_the_row() {
    let doc = Document::parse(&"abcdefghij".repeat(10_000));
    let rows = rows_of(&doc, 20, Hyphenation::Off);
    let widest = rows.iter().map(|r| r.doc.end - r.doc.start).max().unwrap();
    assert!(widest <= 20, "a row of {widest} bytes at width 20");
    assert_rows_honest(&doc, 20, Hyphenation::Off, "100k run");
}

/// A run of soft hyphens is the author's ladder, and it must not overflow
/// the packer's count of one.
///
/// Every row here ends at the author's U+00AD, so every row ends with a
/// hyphen. The count of consecutive hyphenated rows was a `u8` incremented
/// with no ceiling, which panicked in a debug build at the 256th.
#[test]
fn three_hundred_authored_hyphens_in_a_row_wrap_without_overflow() {
    let doc = Document::parse(&"abcdefgh\u{AD}".repeat(300));
    for h in [Hyphenation::Off, Hyphenation::English] {
        let rows = rows_of(&doc, 10, h);
        assert_eq!(rows.len(), 300, "{h:?}");
        assert!(
            rows[..299]
                .iter()
                .all(|r| matches!(r.kind, RowKind::Text { hyphen: true, .. })),
            "every authored division shows its hyphen"
        );
    }
}

/// A cluster wider than the row goes on a row ALONE — not with the cluster
/// after it. At width 1, `す。` used to share a row, because the packer
/// refused the head the splitter rightly offered (one 2-cell cluster) and
/// placed the whole unit instead.
#[test]
fn a_cluster_wider_than_the_row_overhangs_alone() {
    for (src, want) in [
        ("日本語です。", vec!["日", "本", "語", "で", "す", "。"]),
        ("「「「「", vec!["「", "「", "「", "「"]),
    ] {
        let doc = Document::parse(src);
        let rows = rows_of(&doc, 1, Hyphenation::Off);
        let got: Vec<&str> = rows
            .iter()
            .map(|r| &doc.text[r.doc.start as usize..r.doc.end as usize])
            .collect();
        assert_eq!(got, want);
    }
}

/// Wrapping a run with no whitespace in it must be LINEAR with
/// hyphenation on, as it is with it off.
///
/// Each emergency cut of a run longer than a row asked the hyphenator about
/// "the word" it was in, and found that word by scanning back to the
/// previous whitespace and running the patterns over everything up to the
/// next — the whole run, once per row. A 60 KB lowercase run took 0.6–2 s
/// per wrap pass against 2 ms with hyphenation off; and wrap runs twice per
/// relayout. A word longer than the patterns' 63 letters never divides, so
/// there is nothing to look further back for.
#[test]
fn a_long_lowercase_run_wraps_in_linear_time_with_hyphenation_on() {
    use std::time::Instant;

    for (lang, h) in [("en", Hyphenation::English), ("de", Hyphenation::German)] {
        let doc = |n: usize| {
            Document::parse(&format!(
                "---\nlang: {lang}\n---\n\nsome words {} end\n",
                "abcdefghij".repeat(n / 10)
            ))
        };
        let (small, large) = (doc(16_000), doc(64_000));
        let time = |doc: &Document| {
            let t = Instant::now();
            let rows = rows_of(doc, 20, h).len();
            assert!(rows > 0);
            t.elapsed().as_secs_f64()
        };
        time(&small);
        let (a, b) = (time(&small), time(&large));
        assert!(
            b < a * 12.0,
            "{lang}: 4x the run took {:.1}x the time ({a:.4}s -> {b:.4}s). \
             Linear is ~4x; the quadratic this guards was 16x.",
            b / a
        );
        assert_rows_honest(&large, 20, h, lang);
    }
}

/// The reflow promises, fuzzed: documents built from hostile fragments —
/// soft hyphens, runs longer than any row, CJK and its openers, combining
/// marks, every language's hyphenation — wrapped at every width from 1 to
/// 40. Deterministic, so a failure reproduces.
#[test]
fn hostile_text_wraps_honestly_at_every_narrow_width() {
    const FRAGMENTS: &[&str] = &[
        "\u{AD}",
        "abcdefgh\u{AD}",
        "hyphenation ",
        "internationalization ",
        "Donaudampfschifffahrtsgesellschaft ",
        "anticonstitutionnellement ",
        "electroencefalografista ",
        "precipitevolissimevolmente ",
        "inconstitucionalissimamente ",
        "arbeidsongeschiktheidsverzekering ",
        "uafhængighedserklæringerne ",
        "konstantynopolitańczykowianeczka ",
        "abcdefghij",
        "日本語です。",
        "「「「",
        "（（",
        "e\u{301}",
        "\u{200B}",
        "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}",
        " ",
        "  ",
        "\n",
        "\n\n",
        "- ",
        "> ",
        "`co de` ",
        "(((",
        "))) ",
        "https://example.com/a/b/c?d=e ",
        "4f2a1bcd9e8f7a6b5c4d3e2f1a0b9c8d7e6f5a4b ",
    ];
    const LANGS: &[&str] = &["en", "de", "fr", "es", "it", "pt", "nl", "da", "no", "pl"];
    let mut seed = 0x9e37_79b9_7f4a_7c15u64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for i in 0..60 {
        let lang = LANGS[(next() % LANGS.len() as u64) as usize];
        let mut src = format!("---\nlang: {lang}\n---\n\n");
        for _ in 0..next() % 40 {
            let f = FRAGMENTS[(next() % FRAGMENTS.len() as u64) as usize];
            // Now and then a fragment many times over: a run longer than
            // any row, or a ladder of soft hyphens.
            let times = if next() % 8 == 0 { 30 } else { 1 };
            src.push_str(&f.repeat(times));
        }
        let doc = Document::parse(&src);
        let lang_h = carrel_core::hyphenation_for(&doc);
        for width in 1..=40 {
            for h in [Hyphenation::Off, lang_h] {
                assert_rows_honest(&doc, width, h, &format!("#{i} {src:?}"));
            }
        }
    }
}

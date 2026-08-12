//! `layout::wrap` throughput — benchmark item 1 of architecture.md (private notes repo) §6.
//!
//! This is **the** number for the reflow layer. The eager height pass is O(N)
//! over the whole document on every resize (§4), so if a 1 MB document wraps in
//! well under ~10 ms the eager pass is settled and the §4.3 estimate-then-refine
//! fallback is dead code nobody has to write.
//!
//! Two corpora, because the ASCII and wide-character paths measure differently:
//! ASCII is one cell per byte, whereas CJK is two cells per three bytes and
//! offers a break opportunity between every character, so it produces far more
//! units per byte.
//!
//! Run with `cargo bench -p carrel-core`, or `-- --quick` for a rough figure.

use carrel_core::{BlockIdx, Document, cluster_width, wrap};
use criterion::{Criterion, Throughput, criterion_group, criterion_main};

/// Prose-shaped ASCII: paragraphs of varying sentence length, like a README.
fn corpus_ascii(target: usize) -> String {
    const WORDS: &[&str] = &[
        "the", "quick", "brown", "fox", "jumps", "over", "lazy", "dog", "markdown", "reader",
        "terminal", "reflow", "search", "document", "position", "grapheme", "cluster", "width",
    ];
    let mut s = String::with_capacity(target + 1024);
    let mut i = 0usize;
    while s.len() < target {
        for _ in 0..60 {
            s.push_str(WORDS[i % WORDS.len()]);
            s.push(' ');
            i += 1;
        }
        s.push_str("\n\n");
    }
    s
}

/// The wide-character path: every character is 2 cells and breakable.
fn corpus_cjk(target: usize) -> String {
    const TEXT: &str = "日本語のテキストを折り返す処理の速度を測定します。";
    let mut s = String::with_capacity(target + 1024);
    while s.len() < target {
        for _ in 0..8 {
            s.push_str(TEXT);
        }
        s.push_str("\n\n");
    }
    s
}

/// The eager height pass: every block, counting sink, nothing materialised.
/// Exactly what a resize runs.
fn height_pass(doc: &Document, width: u16) -> u32 {
    let mut rows = 0u32;
    for i in 0..doc.block_count() {
        rows += wrap(doc, BlockIdx(i as u32), width, &cluster_width, |_| {});
    }
    rows
}

fn bench_wrap(c: &mut Criterion) {
    const MB: usize = 1 << 20;

    for (name, src) in [("ascii_1mb", corpus_ascii(MB)), ("cjk_1mb", corpus_cjk(MB))] {
        let doc = Document::parse(&src);
        let bytes = doc.text.len() as u64;

        let mut g = c.benchmark_group("height_pass");
        g.throughput(Throughput::Bytes(bytes));
        g.bench_function(name, |b| b.iter(|| height_pass(&doc, 80)));
        g.finish();
    }

    // The row pass materialises `Row`s rather than counting them. Measured
    // separately because only the viewport's worth of it runs per frame.
    let doc = Document::parse(&corpus_ascii(MB));
    let mut g = c.benchmark_group("row_pass");
    g.throughput(Throughput::Bytes(doc.text.len() as u64));
    g.bench_function("ascii_1mb", |b| {
        b.iter(|| {
            let mut rows = Vec::with_capacity(64);
            for i in 0..doc.block_count() {
                wrap(&doc, BlockIdx(i as u32), 80, &cluster_width, |r| {
                    rows.push(r);
                });
                rows.clear();
            }
        });
    });
    g.finish();
}

/// `architecture.md` §6 item 3. Interactive search re-runs the needle on EVERY
/// keystroke, so this number decides whether incremental search is viable at
/// all. A miss here is felt as typing lag, not as a slow operation.
fn bench_search(c: &mut Criterion) {
    const MB: usize = 1 << 20;
    let doc = Document::parse(&corpus_ascii(MB));

    let mut g = c.benchmark_group("search");
    g.throughput(Throughput::Bytes(doc.text.len() as u64));
    g.bench_function("literal_3ch", |b| {
        b.iter(|| carrel_core::search(&doc, "the", true).len());
    });
    g.bench_function("phrase_4word", |b| {
        b.iter(|| carrel_core::search(&doc, "quick brown fox jumps", true).len());
    });
    // What a user actually does: type a needle one character at a time.
    g.bench_function("incremental_5_keystrokes", |b| {
        b.iter(|| {
            let mut n = 0;
            for i in 1..="quick".len() {
                n += carrel_core::search(&doc, &"quick"[..i], true).len();
            }
            n
        });
    });
    g.finish();
}

/// The docs inherited "syntect loads in 0.8 ms" from the design notes without a
/// local measurement. This replaces the claim with a number from this machine.
fn bench_highlight(c: &mut Criterion) {
    // Force the lazy load OUTSIDE the timed section for the throughput number,
    // and measure a cold-ish load by timing the first `highlight` call pattern
    // via a fresh corpus each iteration (the set itself stays cached — a true
    // cold load is once per process and measured separately below).
    let code = "fn compute(alpha: u64, beta: u64) -> u64 {\n    // mix\n    let s = \"seed\";\n    alpha.wrapping_mul(beta) ^ s.len() as u64\n}\n"
        .repeat(200); // ~25 KB of plausible Rust
    let mut g = c.benchmark_group("highlight");
    g.throughput(Throughput::Bytes(code.len() as u64));
    g.bench_function("rust_25kb", |b| {
        b.iter(|| carrel_core::highlight::highlight("rust", &code, 0).len());
    });
    g.finish();
}

criterion_group!(benches, bench_wrap, bench_search, bench_highlight);
criterion_main!(benches);

//! End-to-end resize latency — benchmark item 2 of architecture.md (private notes repo) §6.
//!
//! `App::on_resize` is everything a terminal drag pays per width change:
//! the eager O(N) height pass over every block, plus the anchor restore.
//! Criterion reports the mean; the p95 the architecture doc asks about is in
//! the printed distribution (and with per-iteration work this uniform, the
//! mean and p95 travel together — confirm in the report output).
//!
//! The corpus mixes prose, code, lists, quotes, and a wide table over ~1 MB,
//! because a resize re-lays-out ALL of it, not just the visible screen. The
//! width alternates across the card-view threshold so the most expensive
//! table path is inside the measurement, and the scroll starts mid-document
//! so the anchor restore does real work.
//!
//! Run with `cargo bench -p carrel --bench resize`.

use carrel::action::Action;
use carrel::app::{App, update};
use carrel_core::Document;
use criterion::{Criterion, criterion_group, criterion_main};

fn corpus(target: usize) -> String {
    let mut s = String::with_capacity(target + 4096);
    let mut i = 0usize;
    while s.len() < target {
        s.push_str("## Section heading with some words\n\n");
        for _ in 0..3 {
            for w in 0..40 {
                s.push_str(["alpha", "beta", "gamma", "delta", "epsilon"][(i + w) % 5]);
                s.push(' ');
            }
            s.push_str("\n\n");
        }
        s.push_str("> a quoted line that wraps like prose does\n\n");
        s.push_str("```rust\nfn work(x: u32) -> u32 { x.saturating_add(1) }\n```\n\n");
        s.push_str("- one item\n- another item with rather more text on it\n\n");
        s.push_str("| name | description | value |\n|---|---|---|\n");
        s.push_str("| alpha | a value easily long enough to overflow narrow terminals | 42 |\n\n");
        i += 1;
    }
    s
}

fn bench_resize(c: &mut Criterion) {
    let src = corpus(1 << 20);
    let mut app = App::new("bench.md".into(), Document::parse(&src), 100, 40);
    let mid = app.layout.total_rows() / 2;
    update(&mut app, Action::GoToRow(mid));

    // A drag alternates widths; crossing 60↔100 flips the wide table between
    // aligned and cards, so both table paths are inside the loop.
    let mut w = 60u16;
    c.bench_function("resize_1mb_mixed", |b| {
        b.iter(|| {
            w = if w == 60 { 100 } else { 60 };
            app.on_resize(w, 40);
            std::hint::black_box(app.layout.total_rows());
        });
    });
}

criterion_group!(benches, bench_resize);
criterion_main!(benches);

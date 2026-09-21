//! Budget: parse + validate a whole 10,000-transaction ledger file in
//! ≤50ms (see docs/format.md and the project brief, §7).

use criterion::{criterion_group, criterion_main, Criterion};
use jama_core::model::Directive;
use jama_core::{parser, testkit};

fn bench_parse_and_validate(c: &mut Criterion) {
    let text = testkit::synthetic_beancount_text(10_000);

    c.bench_function("parse+validate 10k transactions", |b| {
        b.iter(|| {
            let directives = parser::parse(std::hint::black_box(&text)).expect("parses");
            for d in &directives {
                if let Directive::Transaction(t) = d {
                    t.resolve_postings().expect("balances");
                }
            }
        });
    });
}

criterion_group!(benches, bench_parse_and_validate);
criterion_main!(benches);

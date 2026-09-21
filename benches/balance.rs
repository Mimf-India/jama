//! Budget: full-tree `balance` over a 10,000-transaction ledger in ≤30ms.

use criterion::{criterion_group, criterion_main, Criterion};
use jama_core::reports::{self, BalanceOptions};
use jama_core::store::Store;
use jama_core::testkit;

fn bench_balance(c: &mut Criterion) {
    let store = Store::open_in_memory().expect("open store");
    testkit::populate_store(&store, 10_000).expect("populate");

    c.bench_function("balance full tree, 10k transactions", |b| {
        b.iter(|| {
            reports::balance(std::hint::black_box(&store), &BalanceOptions::default())
                .expect("balance")
        });
    });
}

criterion_group!(benches, bench_balance);
criterion_main!(benches);

//! Budget: `register` on one account over a 10,000-transaction ledger in
//! ≤20ms.

use criterion::{criterion_group, criterion_main, Criterion};
use jama_core::reports;
use jama_core::store::Store;
use jama_core::testkit;

fn bench_register(c: &mut Criterion) {
    let store = Store::open_in_memory().expect("open store");
    testkit::populate_store(&store, 10_000).expect("populate");

    c.bench_function("register one account, 10k transactions", |b| {
        b.iter(|| {
            reports::register(std::hint::black_box(&store), "assets:checking", None, None)
                .expect("register")
        });
    });
}

criterion_group!(benches, bench_register);
criterion_main!(benches);

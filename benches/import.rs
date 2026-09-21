//! Budget: import 1,000 CSV rows (rule-matched, deduped, inserted) in
//! ≤200ms.

use criterion::{criterion_group, criterion_main, BatchSize, Criterion};
use jama_core::csv_import::{self, Rules};
use jama_core::testkit;
use jama_core::Ledger;

fn bench_import(c: &mut Criterion) {
    let (csv, rules_text) = testkit::synthetic_csv_and_rules(1_000);

    c.bench_function("import 1000 csv rows", |b| {
        b.iter_batched(
            || {
                let dir = tempfile::tempdir().expect("tempdir");
                let ledger = Ledger::init(dir.path()).expect("init ledger");
                let csv_path = dir.path().join("bank.csv");
                std::fs::write(&csv_path, &csv).expect("write csv");
                let rules_path = dir.path().join("rules.toml");
                std::fs::write(&rules_path, &rules_text).expect("write rules");
                (dir, ledger, csv_path, rules_path)
            },
            |(_dir, mut ledger, csv_path, rules_path)| {
                let rules = Rules::load(&rules_path).expect("load rules");
                csv_import::import(&mut ledger, &csv_path, &rules, false).expect("import");
            },
            BatchSize::LargeInput,
        );
    });
}

criterion_group!(benches, bench_import);
criterion_main!(benches);

//! Synthetic ledger generator, for load-testing and manual benchmarking
//! against the §7 performance budget.
//!
//! Usage:
//!
//! ```text
//! cargo run -p jama-core --release --example gen_ledger -- 10000 > /tmp/10k.beancount
//! cargo run -p jama-core --release --example gen_ledger -- 100000 > /tmp/100k.beancount
//! cargo run -p jama-core --release --example gen_ledger -- 1000000 > /tmp/1m.beancount
//! ```

use jama_core::testkit;

fn main() {
    let count: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| {
            eprintln!("usage: gen_ledger <transaction-count>  (e.g. 10000, 100000, 1000000)");
            std::process::exit(1);
        });

    let text = testkit::synthetic_beancount_text(count);
    print!("{text}");
    eprintln!(
        "wrote {count} synthetic transactions ({} bytes) to stdout",
        text.len()
    );
}

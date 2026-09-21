//! Synthetic ledger generation for benchmarks and load testing.
//!
//! Not part of the stable API: shapes here may change without a semver
//! bump. Kept in the library (rather than duplicated in every `benches/`
//! file) because it needs the same model types the rest of `jama-core`
//! does.

use std::collections::BTreeMap;

use rust_decimal::Decimal;

use crate::date::Date;
use crate::error::Result;
use crate::model::{Account, Amount, Directive, Flag, Posting, Transaction};
use crate::store::Store;

const EXPENSE_ACCOUNTS: [&str; 6] = [
    "expenses:cafe",
    "expenses:transport",
    "expenses:groceries",
    "expenses:rent",
    "expenses:utilities",
    "expenses:misc",
];
const PAYEES: [&str; 8] = [
    "Cafe Bateel",
    "Uber",
    "Careem",
    "Panda Hypermarket",
    "Landlord",
    "STC",
    "Amazon",
    "Local Market",
];

/// Generate `n` synthetic, individually-balanced transactions, cycling
/// through a small set of accounts and payees, dated one day apart
/// starting 2020-01-01. Every 50th transaction is income (a salary
/// deposit); the rest are expenses out of `assets:checking`.
pub fn synthetic_transactions(n: usize) -> Vec<Transaction> {
    let mut out = Vec::with_capacity(n);
    let start = Date::from_ymd(2020, 1, 1).expect("valid start date");
    let checking = Account::parse("assets:checking").expect("valid account");
    let salary = Account::parse("income:salary").expect("valid account");

    for i in 0..n {
        let date = start.add_days(i as i64);
        let mut txn = Transaction::new(date, format!("Synthetic transaction {i}"));
        txn.flag = if i % 7 == 0 {
            Flag::Pending
        } else {
            Flag::Cleared
        };
        txn.payee = Some(PAYEES[i % PAYEES.len()].to_string());

        if i % 50 == 0 {
            let amount = Decimal::new(1_500_000, 2); // 15000.00
            txn.postings
                .push(Posting::new(salary.clone(), Amount::new(-amount, "SAR")));
            txn.postings
                .push(Posting::new(checking.clone(), Amount::new(amount, "SAR")));
        } else {
            let cents = 500 + ((i * 37) % 20_000) as i64; // 5.00..205.00
            let amount = Decimal::new(cents, 2);
            let expense = Account::parse(EXPENSE_ACCOUNTS[i % EXPENSE_ACCOUNTS.len()])
                .expect("valid account");
            txn.postings
                .push(Posting::new(checking.clone(), Amount::new(-amount, "SAR")));
            txn.postings
                .push(Posting::new(expense, Amount::new(amount, "SAR")));
        }
        out.push(txn);
    }
    out
}

pub fn synthetic_directives(n: usize) -> Vec<Directive> {
    let mut directives = Vec::with_capacity(n + 2);
    let start = Date::from_ymd(2020, 1, 1).expect("valid start date");
    directives.push(Directive::Open {
        date: start,
        account: Account::parse("assets:checking").unwrap(),
        currencies: vec!["SAR".to_string()],
    });
    directives.push(Directive::Open {
        date: start,
        account: Account::parse("income:salary").unwrap(),
        currencies: vec![],
    });
    directives.extend(
        synthetic_transactions(n)
            .into_iter()
            .map(Directive::Transaction),
    );
    directives
}

pub fn synthetic_beancount_text(n: usize) -> String {
    crate::parser::serialize(&synthetic_directives(n))
}

/// Insert `n` synthetic transactions directly into `store`, bypassing
/// `Ledger` (no text-file sync) for speed — this is what benchmarks that
/// need a pre-populated store use as their setup step.
pub fn populate_store(store: &Store, n: usize) -> Result<()> {
    for txn in synthetic_transactions(n) {
        store.insert_transaction(&txn)?;
    }
    Ok(())
}

/// A synthetic bank CSV of `n` rows plus a matching rules file that
/// categorises every row (so the import benchmark measures real
/// rule-matching + insertion cost, not "everything unmatched").
pub fn synthetic_csv_and_rules(n: usize) -> (String, String) {
    let mut csv = String::from("Date,Amount,Description\n");
    let start = Date::from_ymd(2020, 1, 1).expect("valid start date");
    for i in 0..n {
        let date = start.add_days(i as i64);
        let cents = 500 + ((i * 37) % 20_000) as i64;
        let amount = Decimal::new(-cents, 2);
        let payee = PAYEES[i % PAYEES.len()];
        csv.push_str(&format!("{},{},{}\n", format_ddmmyyyy(date), amount, payee));
    }
    let rules = r#"[source]
date = "Date"
amount = "Amount"
payee = "Description"
date_format = "%d/%m/%Y"
account = "assets:checking"
commodity = "SAR"

[[rule]]
match = "amount < 1000000"
to = "expenses:misc"
"#
    .to_string();
    (csv, rules)
}

fn format_ddmmyyyy(date: Date) -> String {
    format!("{:02}/{:02}/{}", date.day(), date.month(), date.year())
}

/// Every commodity/account combination used above, declared, for
/// scenarios that want a fully-declared (strict-clean) synthetic ledger.
pub fn declared_accounts() -> BTreeMap<&'static str, Vec<&'static str>> {
    let mut m = BTreeMap::new();
    m.insert("assets:checking", vec!["SAR"]);
    m.insert("income:salary", vec![]);
    for a in EXPENSE_ACCOUNTS {
        m.insert(a, vec![]);
    }
    m
}

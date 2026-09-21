//! Read-only reports computed over a [`Store`]: `balance`, `register`,
//! `networth`, and `check`. Pure functions over data already loaded from
//! SQLite — no I/O, no rendering; that's `jama-cli`'s job.

use std::collections::BTreeMap;

use rust_decimal::Decimal;

use crate::date::Date;
use crate::error::Result;
use crate::model::{Account, Amount, Flag};
use crate::store::{ListFilter, Store};

// -- balance -----------------------------------------------------------

#[derive(Debug, Clone)]
pub struct AccountBalance {
    pub account: String,
    pub balances: Vec<Amount>,
}

#[derive(Debug, Default)]
pub struct BalanceOptions {
    pub account_pattern: Option<String>,
    pub depth: Option<usize>,
    pub since: Option<Date>,
    pub until: Option<Date>,
    pub cleared_only: bool,
}

pub fn balance(store: &Store, opts: &BalanceOptions) -> Result<Vec<AccountBalance>> {
    // A lean, purpose-built query (see `Store::posting_rows`) instead of
    // hydrating full `Transaction`s: a full-tree balance only ever needs
    // the (account, number, commodity) triples, so skipping date/flag/
    // payee/narration/tags/links/JSON-meta reconstruction for every
    // posting is what keeps this inside its performance budget on a
    // large ledger.
    let rows = store.posting_rows(opts.since, opts.until, opts.cleared_only)?;

    let mut sums: BTreeMap<String, BTreeMap<String, Decimal>> = BTreeMap::new();
    for (account_str, number, commodity) in rows {
        if let Some(pattern) = &opts.account_pattern {
            if !account_str_matches(&account_str, pattern) {
                continue;
            }
        }
        let key = match opts.depth {
            Some(d) => account_str_prefix(&account_str, d),
            None => account_str,
        };
        *sums
            .entry(key)
            .or_default()
            .entry(commodity)
            .or_insert(Decimal::ZERO) += number;
    }

    Ok(sums
        .into_iter()
        .map(|(account, by_commodity)| AccountBalance {
            account,
            balances: by_commodity
                .into_iter()
                .filter(|(_, n)| !n.is_zero())
                .map(|(c, n)| Amount::new(n, c))
                .collect(),
        })
        .filter(|ab| !ab.balances.is_empty())
        .collect())
}

/// Equivalent to [`Account::matches`], operating on the raw string a
/// posting row already carries (it was validated once, at insert time —
/// re-parsing it into an `Account` for every row in a large ledger is
/// wasted work a hot report path shouldn't pay for).
fn account_str_matches(account: &str, pattern: &str) -> bool {
    let pattern = pattern.strip_suffix(':').unwrap_or(pattern);
    if pattern.is_empty() {
        return true;
    }
    account == pattern || account.starts_with(&format!("{pattern}:"))
}

/// Equivalent to [`Account::prefix`], on a raw string.
fn account_str_prefix(account: &str, depth: usize) -> String {
    account
        .splitn(depth + 1, ':')
        .take(depth)
        .collect::<Vec<_>>()
        .join(":")
}

// -- register ------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct RegisterEntry {
    pub transaction_id: i64,
    pub date: Date,
    pub flag: Flag,
    pub payee: Option<String>,
    pub narration: String,
    pub account: String,
    pub amount: Amount,
    pub running_balance: Vec<Amount>,
}

pub fn register(
    store: &Store,
    account_pattern: &str,
    since: Option<Date>,
    until: Option<Date>,
) -> Result<Vec<RegisterEntry>> {
    // See `Store::register_rows`: this fetches exactly the matching
    // postings, joined with just the transaction fields register
    // displays, already ordered — no full-ledger hydration.
    let rows = store.register_rows(account_pattern, since, until)?;

    let mut running: BTreeMap<String, Decimal> = BTreeMap::new();
    let mut entries = Vec::with_capacity(rows.len());
    for (transaction_id, date, flag, payee, narration, account, number, commodity) in rows {
        let bal = running.entry(commodity.clone()).or_insert(Decimal::ZERO);
        *bal += number;
        let running_balance: Vec<Amount> = running
            .iter()
            .map(|(c, n)| Amount::new(*n, c.clone()))
            .collect();
        entries.push(RegisterEntry {
            transaction_id,
            date,
            flag,
            payee,
            narration,
            account,
            amount: Amount::new(number, commodity),
            running_balance,
        });
    }
    Ok(entries)
}

// -- networth ------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct NetWorthPoint {
    pub as_of: Date,
    pub balances: Vec<Amount>,
}

pub fn networth(
    store: &Store,
    since: Option<Date>,
    until: Option<Date>,
    monthly: bool,
) -> Result<Vec<NetWorthPoint>> {
    let filter = ListFilter {
        since,
        until,
        ..Default::default()
    };
    let mut txns = store.list_transactions(&filter)?;
    txns.sort_by_key(|t| (t.date, t.id));

    if !monthly {
        let totals = accumulate_networth(&txns, None);
        let as_of = txns.last().map(|t| t.date).unwrap_or_else(Date::today);
        return Ok(vec![NetWorthPoint {
            as_of,
            balances: totals,
        }]);
    }

    // One point per calendar month that has at least one transaction,
    // valued as of the last day covered (start of the *next* transaction's
    // month minus one day is unnecessary — we just snapshot after each
    // month's transactions are applied).
    let mut points = Vec::new();
    let mut running: BTreeMap<String, Decimal> = BTreeMap::new();
    let mut current_month = None;
    for t in &txns {
        let month_start = t.date.month_start();
        if let Some(prev_month) = current_month {
            if prev_month != month_start {
                points.push(NetWorthPoint {
                    as_of: prev_month,
                    balances: running
                        .iter()
                        .map(|(c, n)| Amount::new(*n, c.clone()))
                        .collect(),
                });
            }
        }
        current_month = Some(month_start);
        for p in &t.postings {
            if is_networth_account(&p.account) {
                if let Some(amount) = &p.amount {
                    *running
                        .entry(amount.commodity.clone())
                        .or_insert(Decimal::ZERO) += amount.number;
                }
            }
        }
    }
    if let Some(month) = current_month {
        points.push(NetWorthPoint {
            as_of: month,
            balances: running
                .into_iter()
                .filter(|(_, n)| !n.is_zero())
                .map(|(c, n)| Amount::new(n, c))
                .collect(),
        });
    }
    Ok(points)
}

fn is_networth_account(account: &Account) -> bool {
    matches!(account.root(), "assets" | "liabilities")
}

fn accumulate_networth(txns: &[crate::model::Transaction], _cutoff: Option<Date>) -> Vec<Amount> {
    let mut sums: BTreeMap<String, Decimal> = BTreeMap::new();
    for t in txns {
        for p in &t.postings {
            if is_networth_account(&p.account) {
                if let Some(amount) = &p.amount {
                    *sums
                        .entry(amount.commodity.clone())
                        .or_insert(Decimal::ZERO) += amount.number;
                }
            }
        }
    }
    sums.into_iter()
        .filter(|(_, n)| !n.is_zero())
        .map(|(c, n)| Amount::new(n, c))
        .collect()
}

// -- check ---------------------------------------------------------------

#[derive(Debug, Default)]
pub struct CheckReport {
    pub transactions_checked: usize,
    pub imbalances: Vec<String>,
    pub failed_assertions: Vec<String>,
    pub undeclared_accounts: Vec<String>,
}

impl CheckReport {
    pub fn is_clean(&self) -> bool {
        self.imbalances.is_empty() && self.failed_assertions.is_empty()
    }
}

pub fn check(store: &Store) -> Result<CheckReport> {
    let mut report = CheckReport::default();
    let txns = store.all_transactions()?;
    report.transactions_checked = txns.len();

    let mut undeclared: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for t in &txns {
        if let Err(imb) = t.resolve_postings() {
            report.imbalances.push(imb.to_string());
        }
        for p in &t.postings {
            if !store.is_account_declared(p.account.as_str())? {
                undeclared.insert(p.account.to_string());
            }
        }
    }
    report.undeclared_accounts = undeclared.into_iter().collect();

    // Balance assertions: recompute the running balance for the account up
    // to (and including) the assertion date and compare.
    for (date, account, expected) in store.list_balance_assertions()? {
        let mut running = Decimal::ZERO;
        for t in &txns {
            if t.date > date {
                continue;
            }
            for p in &t.postings {
                if p.account == account {
                    if let Some(a) = &p.amount {
                        if a.commodity == expected.commodity {
                            running += a.number;
                        }
                    }
                }
            }
        }
        if running != expected.number {
            report.failed_assertions.push(format!(
                "{date} {account}: expected {expected}, got {running} {}",
                expected.commodity
            ));
        }
    }

    Ok(report)
}

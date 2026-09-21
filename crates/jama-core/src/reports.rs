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
    let filter = ListFilter {
        since: opts.since,
        until: opts.until,
        ..Default::default()
    };
    let txns = store.list_transactions(&filter)?;

    let mut sums: BTreeMap<String, BTreeMap<String, Decimal>> = BTreeMap::new();
    for t in &txns {
        if opts.cleared_only && t.flag != Flag::Cleared {
            continue;
        }
        for p in &t.postings {
            if let Some(pattern) = &opts.account_pattern {
                if !p.account.matches(pattern) {
                    continue;
                }
            }
            let key = match opts.depth {
                Some(d) => p.account.prefix(d),
                None => p.account.as_str().to_string(),
            };
            if let Some(amount) = &p.amount {
                *sums
                    .entry(key)
                    .or_default()
                    .entry(amount.commodity.clone())
                    .or_insert(Decimal::ZERO) += amount.number;
            }
        }
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
    let filter = ListFilter {
        since,
        until,
        ..Default::default()
    };
    let mut txns = store.list_transactions(&filter)?;
    txns.sort_by_key(|t| (t.date, t.id));

    let mut running: BTreeMap<String, Decimal> = BTreeMap::new();
    let mut entries = Vec::new();
    for t in &txns {
        for p in &t.postings {
            if !p.account.matches(account_pattern) {
                continue;
            }
            if let Some(amount) = &p.amount {
                let bal = running
                    .entry(amount.commodity.clone())
                    .or_insert(Decimal::ZERO);
                *bal += amount.number;
                let running_balance: Vec<Amount> = running
                    .iter()
                    .map(|(c, n)| Amount::new(*n, c.clone()))
                    .collect();
                entries.push(RegisterEntry {
                    transaction_id: t.id.unwrap_or_default(),
                    date: t.date,
                    flag: t.flag,
                    payee: t.payee.clone(),
                    narration: t.narration.clone(),
                    account: p.account.to_string(),
                    amount: amount.clone(),
                    running_balance,
                });
            }
        }
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

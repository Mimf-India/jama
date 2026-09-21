//! The JAMA data model: accounts, commodities, postings, transactions and
//! the directives a ledger file is made of.
//!
//! Amounts are always [`rust_decimal::Decimal`] — fixed-point, exact,
//! never a float — so money never drifts from repeated arithmetic.

use std::collections::BTreeMap;
use std::fmt;

use rust_decimal::Decimal;

use crate::date::Date;

/// The five account roots every account must live under.
pub const ROOTS: [&str; 5] = ["assets", "liabilities", "equity", "income", "expenses"];

/// A colon-separated account path, e.g. `assets:checking:alrajhi`.
///
/// Validated to start with one of the five roots and contain only
/// non-empty, colon-separated segments.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Account(String);

impl Account {
    pub fn parse(s: &str) -> Result<Self, String> {
        let s = s.trim();
        if s.is_empty() {
            return Err("account name is empty".to_string());
        }
        let segments: Vec<&str> = s.split(':').collect();
        if segments.iter().any(|seg| seg.is_empty()) {
            return Err(format!("account {s:?} has an empty segment"));
        }
        if !ROOTS.contains(&segments[0]) {
            return Err(format!(
                "account {s:?} must start with one of: {}",
                ROOTS.join(", ")
            ));
        }
        for seg in &segments {
            if !seg
                .chars()
                .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
            {
                return Err(format!(
                    "account segment {seg:?} in {s:?} has invalid characters"
                ));
            }
        }
        Ok(Self(s.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn root(&self) -> &str {
        self.0.split(':').next().unwrap_or("")
    }

    pub fn depth(&self) -> usize {
        self.0.split(':').count()
    }

    /// Truncate to the first `depth` colon-separated segments.
    pub fn prefix(&self, depth: usize) -> String {
        self.0
            .splitn(depth + 1, ':')
            .take(depth)
            .collect::<Vec<_>>()
            .join(":")
    }

    /// Whether this account is `pattern` itself or a descendant of it.
    /// An empty pattern matches everything.
    pub fn matches(&self, pattern: &str) -> bool {
        if pattern.is_empty() {
            return true;
        }
        self.0 == pattern || self.0.starts_with(&format!("{pattern}:"))
    }

    /// Loose substring match against pattern, case-insensitive — used for
    /// `--account` filters where the user just wants "anything with this in it".
    pub fn contains_loose(&self, pattern: &str) -> bool {
        self.0.to_lowercase().contains(&pattern.to_lowercase())
    }
}

impl fmt::Display for Account {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A commodity code, e.g. `SAR`, `USD`, `BTC`, plus its declared display
/// precision (number of decimal places to render).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommodityInfo {
    pub code: String,
    pub precision: u8,
}

impl CommodityInfo {
    pub fn new(code: impl Into<String>, precision: u8) -> Self {
        Self {
            code: code.into(),
            precision,
        }
    }
}

/// A decimal quantity tagged with its commodity. Multi-currency amounts are
/// never silently summed: arithmetic across amounts always groups by
/// commodity first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Amount {
    pub number: Decimal,
    pub commodity: String,
}

impl Amount {
    pub fn new(number: Decimal, commodity: impl Into<String>) -> Self {
        Self {
            number,
            commodity: commodity.into(),
        }
    }
}

impl fmt::Display for Amount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.number, self.commodity)
    }
}

/// An optional cost basis or price attached to a posting (`{cost}` /
/// `@ price` in Beancount notation).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CostOrPrice {
    pub amount: Amount,
    /// `true` for a `{cost}` lot annotation, `false` for an `@ price`.
    pub is_cost: bool,
}

/// One leg of a transaction. `amount` is `None` for the single posting per
/// transaction that is allowed to be elided and auto-balanced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Posting {
    pub account: Account,
    pub amount: Option<Amount>,
    pub cost_or_price: Option<CostOrPrice>,
    pub meta: BTreeMap<String, String>,
}

impl Posting {
    pub fn new(account: Account, amount: Amount) -> Self {
        Self {
            account,
            amount: Some(amount),
            cost_or_price: None,
            meta: BTreeMap::new(),
        }
    }

    pub fn elided(account: Account) -> Self {
        Self {
            account,
            amount: None,
            cost_or_price: None,
            meta: BTreeMap::new(),
        }
    }
}

/// Transaction clearing state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flag {
    /// `*` — cleared / reconciled.
    Cleared,
    /// `!` — pending / needs attention.
    Pending,
}

impl Flag {
    pub fn as_char(&self) -> char {
        match self {
            Flag::Cleared => '*',
            Flag::Pending => '!',
        }
    }

    pub fn parse(c: char) -> Result<Self, String> {
        match c {
            '*' => Ok(Flag::Cleared),
            '!' => Ok(Flag::Pending),
            other => Err(format!(
                "unknown transaction flag {other:?} (expected * or !)"
            )),
        }
    }
}

/// A double-entry transaction: a date, a flag, who and what, and two or
/// more balancing postings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transaction {
    /// Database row id, once persisted. `None` for a transaction not yet
    /// saved.
    pub id: Option<i64>,
    pub date: Date,
    pub flag: Flag,
    pub payee: Option<String>,
    pub narration: String,
    pub tags: Vec<String>,
    pub links: Vec<String>,
    pub postings: Vec<Posting>,
    pub meta: BTreeMap<String, String>,
}

impl Transaction {
    pub fn new(date: Date, narration: impl Into<String>) -> Self {
        Self {
            id: None,
            date,
            flag: Flag::Cleared,
            payee: None,
            narration: narration.into(),
            tags: Vec::new(),
            links: Vec::new(),
            postings: Vec::new(),
            meta: BTreeMap::new(),
        }
    }

    /// Resolve any single elided posting per commodity group and verify the
    /// transaction balances to zero in every commodity. Returns the fully
    /// resolved postings (with elided amounts filled in) or a descriptive
    /// [`Imbalance`] error naming the offending postings.
    pub fn balanced_postings(&self) -> Result<Vec<Amount>, Imbalance> {
        if self.postings.len() < 2 {
            return Err(Imbalance {
                narration: self.narration.clone(),
                date: self.date,
                details: "transaction has fewer than 2 postings".to_string(),
            });
        }

        // Group explicit amounts by commodity; track elided postings.
        let mut sums: BTreeMap<String, Decimal> = BTreeMap::new();
        let mut elided: Vec<&Account> = Vec::new();
        for p in &self.postings {
            match &p.amount {
                Some(a) => *sums.entry(a.commodity.clone()).or_insert(Decimal::ZERO) += a.number,
                None => elided.push(&p.account),
            }
        }

        if elided.len() > 1 {
            return Err(Imbalance {
                narration: self.narration.clone(),
                date: self.date,
                details: format!(
                    "{} postings have no amount; only one is allowed",
                    elided.len()
                ),
            });
        }

        let mut resolved = Vec::new();
        if elided.len() == 1 {
            // The elided posting absorbs whichever single commodity is
            // currently out of balance. If more than one commodity is
            // unbalanced, we can't tell which one it should cover.
            let unbalanced: Vec<(&String, &Decimal)> =
                sums.iter().filter(|(_, v)| !v.is_zero()).collect();
            match unbalanced.len() {
                0 => {
                    // Already balanced; elided posting is a true zero (unusual
                    // but not an error) — pick the first known commodity if any.
                    if let Some((commodity, _)) = sums.iter().next() {
                        resolved.push(Amount::new(Decimal::ZERO, commodity.clone()));
                    }
                }
                1 => {
                    let (commodity, sum) = unbalanced[0];
                    resolved.push(Amount::new(-*sum, commodity.clone()));
                }
                _ => {
                    return Err(Imbalance {
                        narration: self.narration.clone(),
                        date: self.date,
                        details: "elided posting is ambiguous across multiple commodities"
                            .to_string(),
                    });
                }
            }
        } else {
            let imbalanced: Vec<String> = sums
                .iter()
                .filter(|(_, v)| !v.is_zero())
                .map(|(c, v)| format!("{v} {c}"))
                .collect();
            if !imbalanced.is_empty() {
                return Err(Imbalance {
                    narration: self.narration.clone(),
                    date: self.date,
                    details: format!("does not balance to zero: {}", imbalanced.join(", ")),
                });
            }
        }

        for (commodity, sum) in sums {
            resolved.push(Amount::new(sum, commodity));
        }
        Ok(resolved)
    }

    pub fn is_balanced(&self) -> bool {
        self.balanced_postings().is_ok()
    }

    /// Like [`Self::balanced_postings`], but returns the transaction's own
    /// postings with the (at most one) elided amount filled in, in their
    /// original order — ready to persist.
    pub fn resolve_postings(&self) -> Result<Vec<Posting>, Imbalance> {
        let mut sums: BTreeMap<String, Decimal> = BTreeMap::new();
        let mut elided_index: Option<usize> = None;
        for (i, p) in self.postings.iter().enumerate() {
            match &p.amount {
                Some(a) => *sums.entry(a.commodity.clone()).or_insert(Decimal::ZERO) += a.number,
                None => {
                    if elided_index.is_some() {
                        return Err(Imbalance {
                            narration: self.narration.clone(),
                            date: self.date,
                            details: "more than one posting has no amount".to_string(),
                        });
                    }
                    elided_index = Some(i);
                }
            }
        }

        let mut postings = self.postings.clone();
        match elided_index {
            None => {
                let imbalanced: Vec<String> = sums
                    .iter()
                    .filter(|(_, v)| !v.is_zero())
                    .map(|(c, v)| format!("{v} {c}"))
                    .collect();
                if !imbalanced.is_empty() {
                    return Err(Imbalance {
                        narration: self.narration.clone(),
                        date: self.date,
                        details: format!("does not balance to zero: {}", imbalanced.join(", ")),
                    });
                }
            }
            Some(idx) => {
                let unbalanced: Vec<(&String, &Decimal)> =
                    sums.iter().filter(|(_, v)| !v.is_zero()).collect();
                let fill = match unbalanced.len() {
                    0 => sums
                        .iter()
                        .next()
                        .map(|(c, _)| Amount::new(Decimal::ZERO, c.clone())),
                    1 => {
                        let (commodity, sum) = unbalanced[0];
                        Some(Amount::new(-*sum, commodity.clone()))
                    }
                    _ => {
                        return Err(Imbalance {
                            narration: self.narration.clone(),
                            date: self.date,
                            details: "elided posting is ambiguous across multiple commodities"
                                .to_string(),
                        })
                    }
                };
                postings[idx].amount = fill;
            }
        }
        Ok(postings)
    }
}

/// The transaction doesn't sum to zero per commodity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Imbalance {
    pub date: Date,
    pub narration: String,
    pub details: String,
}

impl fmt::Display for Imbalance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} \"{}\" {}", self.date, self.narration, self.details)
    }
}

/// A ledger-file directive. `Transaction` is the common case; the rest
/// declare accounts, commodities, and checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Directive {
    Transaction(Transaction),
    /// `YYYY-MM-DD open assets:checking SAR,USD`
    Open {
        date: Date,
        account: Account,
        currencies: Vec<String>,
    },
    /// `YYYY-MM-DD close assets:checking`
    Close {
        date: Date,
        account: Account,
    },
    /// `YYYY-MM-DD balance assets:checking 1000.00 SAR`
    Balance {
        date: Date,
        account: Account,
        amount: Amount,
    },
    /// `YYYY-MM-DD pad assets:checking equity:opening-balances`
    Pad {
        date: Date,
        account: Account,
        source: Account,
    },
    /// `YYYY-MM-DD commodity SAR` with an optional `precision` metadatum.
    Commodity {
        date: Date,
        info: CommodityInfo,
    },
}

impl Directive {
    pub fn date(&self) -> Date {
        match self {
            Directive::Transaction(t) => t.date,
            Directive::Open { date, .. }
            | Directive::Close { date, .. }
            | Directive::Balance { date, .. }
            | Directive::Pad { date, .. }
            | Directive::Commodity { date, .. } => *date,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn acct(s: &str) -> Account {
        Account::parse(s).unwrap()
    }

    #[test]
    fn account_requires_known_root() {
        assert!(Account::parse("assets:checking").is_ok());
        assert!(Account::parse("wallet:cash").is_err());
        assert!(Account::parse("assets:").is_err());
    }

    #[test]
    fn account_prefix_and_matches() {
        let a = acct("assets:checking:alrajhi");
        assert_eq!(a.prefix(2), "assets:checking");
        assert!(a.matches("assets"));
        assert!(a.matches("assets:checking"));
        assert!(!a.matches("assets:savings"));
    }

    #[test]
    fn simple_two_posting_transaction_balances() {
        let date = Date::from_ymd(2026, 1, 1).unwrap();
        let mut t = Transaction::new(date, "Coffee");
        t.postings.push(Posting::new(
            acct("assets:checking"),
            Amount::new(Decimal::from_str("-12.50").unwrap(), "SAR"),
        ));
        t.postings.push(Posting::new(
            acct("expenses:cafe"),
            Amount::new(Decimal::from_str("12.50").unwrap(), "SAR"),
        ));
        assert!(t.is_balanced());
    }

    #[test]
    fn elided_posting_absorbs_the_difference() {
        let date = Date::from_ymd(2026, 1, 1).unwrap();
        let mut t = Transaction::new(date, "Coffee");
        t.postings.push(Posting::new(
            acct("assets:checking"),
            Amount::new(Decimal::from_str("-12.50").unwrap(), "SAR"),
        ));
        t.postings.push(Posting::elided(acct("expenses:cafe")));
        let resolved = t.balanced_postings().unwrap();
        let cafe = resolved.iter().find(|a| a.commodity == "SAR").unwrap();
        assert_eq!(cafe.number, Decimal::from_str("12.50").unwrap());
    }

    #[test]
    fn unbalanced_transaction_is_rejected() {
        let date = Date::from_ymd(2026, 1, 1).unwrap();
        let mut t = Transaction::new(date, "Oops");
        t.postings.push(Posting::new(
            acct("assets:checking"),
            Amount::new(Decimal::from_str("-12.50").unwrap(), "SAR"),
        ));
        t.postings.push(Posting::new(
            acct("expenses:cafe"),
            Amount::new(Decimal::from_str("10.00").unwrap(), "SAR"),
        ));
        assert!(t.balanced_postings().is_err());
    }

    #[test]
    fn different_commodities_never_silently_sum() {
        let date = Date::from_ymd(2026, 1, 1).unwrap();
        let mut t = Transaction::new(date, "FX");
        t.postings.push(Posting::new(
            acct("assets:checking"),
            Amount::new(Decimal::from_str("-100").unwrap(), "USD"),
        ));
        t.postings.push(Posting::new(
            acct("assets:savings"),
            Amount::new(Decimal::from_str("100").unwrap(), "SAR"),
        ));
        // Neither commodity balances on its own, and there's no elided
        // posting to absorb the difference, so this must be rejected.
        assert!(t.balanced_postings().is_err());
    }
}

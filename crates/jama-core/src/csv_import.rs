//! CSV bank-statement import: a human-editable TOML rules file maps raw
//! rows to postings, idempotently (re-importing the same file skips rows
//! already seen) and matches unmatched rows against a tiny condition DSL.
//!
//! Rules file shape:
//!
//! ```toml
//! [source]
//! date = "Date"
//! amount = "Amount"
//! payee = "Description"
//! date_format = "%d/%m/%Y"
//! account = "assets:checking"
//!
//! [[rule]]
//! match = "payee ~ /uber|careem/i"
//! to = "expenses:transport"
//! ```
//!
//! `~` matching is intentionally a *lite* regex: only literal
//! alternation (`a|b|c`), optionally case-insensitive with the `i` flag —
//! enough for keyword-matching bank descriptions without pulling in a full
//! regex engine.

use std::collections::BTreeMap;
use std::path::Path;

use rust_decimal::Decimal;
use serde::Deserialize;
use std::str::FromStr;

use crate::date::Date;
use crate::error::{JamaError, Result};
use crate::ledger::Ledger;
use crate::model::{Account, Amount, Flag, Posting, Transaction};
use crate::store::Store;

#[derive(Debug, Deserialize)]
struct RawRulesFile {
    source: SourceConfig,
    #[serde(default, rename = "rule")]
    rules: Vec<RuleConfig>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct SourceConfig {
    pub date: String,
    pub amount: String,
    pub payee: String,
    pub date_format: String,
    pub account: String,
    /// This statement's commodity, e.g. `"SAR"` or `"USD"`. Optional: if
    /// omitted, JAMA falls back to the source account's declared
    /// currency, then the ledger's configured default — see
    /// [`crate::ledger::Ledger::resolve_commodity`]. There is no
    /// hardcoded fallback currency; an import with none of these will
    /// fail with an actionable error rather than silently guessing.
    #[serde(default)]
    pub commodity: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
struct RuleConfig {
    #[serde(rename = "match")]
    match_expr: String,
    to: String,
}

pub struct Rules {
    pub source: SourceConfig,
    compiled: Vec<(Cond, String)>,
}

impl Rules {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).map_err(|e| JamaError::Io {
            context: format!("reading rules file {}", path.display()),
            source: e,
        })?;
        let raw: RawRulesFile = toml::from_str(&text)
            .map_err(|e| JamaError::Import(format!("{}: {e}", path.display())))?;
        let mut compiled = Vec::new();
        for r in raw.rules {
            let cond = parse_expr(&r.match_expr).map_err(|e| {
                JamaError::Import(format!("bad `match` expression {:?}: {e}", r.match_expr))
            })?;
            compiled.push((cond, r.to));
        }
        Ok(Self {
            source: raw.source,
            compiled,
        })
    }

    /// The destination account for the first matching rule, if any.
    pub fn destination_for(&self, row: &CsvRow) -> Option<&str> {
        self.compiled
            .iter()
            .find(|(cond, _)| cond.eval(row))
            .map(|(_, to)| to.as_str())
    }

    /// Append a learned rule (`payee ~ /literal/i` -> `to`) to the rules
    /// file on disk, so future imports categorise it automatically.
    pub fn append_learned_rule(path: &Path, payee_literal: &str, to: &str) -> Result<()> {
        let escaped = payee_literal.replace('/', "\\/");
        let block = format!("\n[[rule]]\nmatch = \"payee ~ /{escaped}/i\"\nto = \"{to}\"\n");
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|e| JamaError::Io {
                context: format!("appending to {}", path.display()),
                source: e,
            })?;
        f.write_all(block.as_bytes()).map_err(|e| JamaError::Io {
            context: "writing learned rule".to_string(),
            source: e,
        })
    }
}

/// One parsed CSV row, with the fields the rules DSL can reference.
#[derive(Debug, Clone)]
pub struct CsvRow {
    pub date: Date,
    pub amount: Decimal,
    pub payee: String,
    pub raw: BTreeMap<String, String>,
    pub dedupe_hash: String,
}

impl CsvRow {
    fn field_str(&self, field: &str) -> Option<String> {
        match field {
            "payee" | "narration" | "description" => Some(self.payee.clone()),
            "date" => Some(self.date.to_string()),
            other => self.raw.get(other).cloned(),
        }
    }

    fn field_number(&self, field: &str) -> Option<Decimal> {
        match field {
            "amount" => Some(self.amount),
            other => self.raw.get(other).and_then(|s| clean_number(s).ok()),
        }
    }
}

pub struct ImportOutcome {
    pub total_rows: usize,
    pub imported: usize,
    pub skipped_duplicates: usize,
    pub unmatched: Vec<CsvRow>,
}

/// Read and parse every row of the CSV against the rules' `[source]`
/// column mapping, without touching the store.
pub fn parse_csv_rows(csv_path: &Path, source: &SourceConfig) -> Result<Vec<CsvRow>> {
    let file_name = csv_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("import.csv")
        .to_string();
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .from_path(csv_path)
        .map_err(|e| JamaError::Import(format!("opening {}: {e}", csv_path.display())))?;

    let headers = reader
        .headers()
        .map_err(|e| JamaError::Import(format!("reading headers: {e}")))?
        .clone();

    let mut rows = Vec::new();
    for (line_no, result) in reader.records().enumerate() {
        let record = result.map_err(|e| JamaError::Import(format!("row {}: {e}", line_no + 2)))?;
        let mut raw = BTreeMap::new();
        for (h, v) in headers.iter().zip(record.iter()) {
            raw.insert(h.to_string(), v.to_string());
        }
        let date_str = raw.get(&source.date).cloned().unwrap_or_default();
        let amount_str = raw.get(&source.amount).cloned().unwrap_or_default();
        let payee = raw.get(&source.payee).cloned().unwrap_or_default();

        let date = Date::parse_with_format(&date_str, &source.date_format)
            .map_err(|e| JamaError::Import(format!("row {}: {e}", line_no + 2)))?;
        let amount = clean_number(&amount_str).map_err(|e| {
            JamaError::Import(format!(
                "row {}: bad amount {amount_str:?}: {e}",
                line_no + 2
            ))
        })?;

        let dedupe_hash = dedupe_key(&date, &amount, &payee, &file_name);
        rows.push(CsvRow {
            date,
            amount,
            payee,
            raw,
            dedupe_hash,
        });
    }
    Ok(rows)
}

/// Import a CSV file against a rules file: dedupe against already-seen
/// rows, apply rules to auto-categorise, and insert matched rows as
/// transactions. Unmatched rows are returned for interactive
/// categorisation by the caller. In `dry_run` mode nothing is written.
pub fn import(
    ledger: &mut Ledger,
    csv_path: &Path,
    rules: &Rules,
    dry_run: bool,
) -> Result<ImportOutcome> {
    let rows = parse_csv_rows(csv_path, &rules.source)?;
    let source_account = Account::parse(&rules.source.account)
        .map_err(|e| JamaError::Import(format!("bad [source].account: {e}")))?;
    let commodity = resolve_commodity(ledger, rules, &source_account)?;

    let mut outcome = ImportOutcome {
        total_rows: rows.len(),
        imported: 0,
        skipped_duplicates: 0,
        unmatched: Vec::new(),
    };

    if dry_run {
        for row in rows {
            if ledger.store.has_import_hash(&row.dedupe_hash)? {
                outcome.skipped_duplicates += 1;
                continue;
            }
            match rules.destination_for(&row) {
                Some(_) => outcome.imported += 1,
                None => outcome.unmatched.push(row),
            }
        }
        return Ok(outcome);
    }

    // The whole batch runs inside one SQLite transaction: inserting rows
    // one at a time, each auto-committing (and fsyncing) on its own, is
    // what made a 1,000-row import take ~4.5s instead of the ~200ms
    // budget in §7 of the brief.
    let (mut imported, mut skipped, mut unmatched) = (0usize, 0usize, Vec::new());
    ledger.store.with_transaction(|store| {
        for row in &rows {
            if store.has_import_hash(&row.dedupe_hash)? {
                skipped += 1;
                continue;
            }
            match rules.destination_for(row) {
                Some(destination) => {
                    insert_row_in_store(store, &source_account, destination, &commodity, row)?;
                    imported += 1;
                }
                None => unmatched.push(row.clone()),
            }
        }
        Ok(())
    })?;
    outcome.imported = imported;
    outcome.skipped_duplicates = skipped;
    outcome.unmatched = unmatched;

    ledger.sync_text_file()?;
    Ok(outcome)
}

/// Resolve the commodity for an entire import: an explicit
/// `[source].commodity` in the rules file, else the source account's
/// declared currency, else the ledger's configured default — see
/// `Ledger::resolve_commodity`. A whole bank statement file is assumed to
/// be in one commodity; there is no per-row override in v0.
pub fn resolve_commodity(
    ledger: &Ledger,
    rules: &Rules,
    source_account: &Account,
) -> Result<String> {
    ledger.resolve_commodity(rules.source.commodity.as_deref(), &[source_account])
}

/// Insert a single categorised row as a transaction and record its dedupe
/// hash — used for rows the caller categorised interactively, one at a
/// time, after `import()`'s bulk pass left them unmatched.
pub fn insert_row(
    ledger: &mut Ledger,
    source_account: &Account,
    destination: &str,
    commodity: &str,
    row: &CsvRow,
) -> Result<i64> {
    insert_row_in_store(&ledger.store, source_account, destination, commodity, row)
}

/// The actual insert, against a `Store` directly rather than a `Ledger` —
/// so the bulk importer above can run many of these inside a single
/// SQLite transaction instead of one transaction (and fsync) per row.
fn insert_row_in_store(
    store: &Store,
    source_account: &Account,
    destination: &str,
    commodity: &str,
    row: &CsvRow,
) -> Result<i64> {
    let destination = Account::parse(destination).map_err(JamaError::Import)?;
    let mut txn = Transaction::new(row.date, row.payee.clone());
    txn.flag = Flag::Cleared;
    txn.postings.push(Posting::new(
        source_account.clone(),
        Amount::new(row.amount, commodity),
    ));
    txn.postings.push(Posting::elided(destination));
    let resolved = txn.resolve_postings().map_err(JamaError::Imbalance)?;
    txn.postings = resolved;
    let id = store.insert_transaction(&txn)?;
    store.record_import_hash(&row.dedupe_hash, id)?;
    Ok(id)
}

fn clean_number(s: &str) -> std::result::Result<Decimal, String> {
    let cleaned: String = s
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '.' || *c == '-' || *c == '+')
        .collect();
    Decimal::from_str(&cleaned).map_err(|e| e.to_string())
}

fn dedupe_key(date: &Date, amount: &Decimal, payee: &str, source_file: &str) -> String {
    let combined = format!("{date}|{amount}|{payee}|{source_file}");
    format!("{:016x}", fnv1a64(combined.as_bytes()))
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

// -- tiny condition DSL ----------------------------------------------------

#[derive(Debug, Clone)]
enum CondValue {
    Regex(Vec<String>, bool), // alternatives, case_insensitive
    Number(Decimal),
    Str(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CondOp {
    Match,
    Lt,
    Gt,
    Le,
    Ge,
    Eq,
    Ne,
}

#[derive(Debug, Clone)]
struct Term {
    field: String,
    op: CondOp,
    value: CondValue,
}

impl Term {
    fn eval(&self, row: &CsvRow) -> bool {
        match &self.value {
            CondValue::Regex(alts, ci) => {
                let hay = row.field_str(&self.field).unwrap_or_default();
                let hay = if *ci { hay.to_lowercase() } else { hay };
                alts.iter().any(|alt| {
                    let needle = if *ci { alt.to_lowercase() } else { alt.clone() };
                    hay.contains(&needle)
                })
            }
            CondValue::Number(n) => {
                let Some(field_val) = row.field_number(&self.field) else {
                    return false;
                };
                match self.op {
                    CondOp::Lt => field_val < *n,
                    CondOp::Gt => field_val > *n,
                    CondOp::Le => field_val <= *n,
                    CondOp::Ge => field_val >= *n,
                    CondOp::Eq => field_val == *n,
                    CondOp::Ne => field_val != *n,
                    CondOp::Match => false,
                }
            }
            CondValue::Str(s) => {
                let Some(field_val) = row.field_str(&self.field) else {
                    return false;
                };
                match self.op {
                    CondOp::Eq => field_val == *s,
                    CondOp::Ne => field_val != *s,
                    _ => false,
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LogicOp {
    And,
    Or,
}

#[derive(Debug, Clone)]
struct Cond {
    first: Term,
    rest: Vec<(LogicOp, Term)>,
}

impl Cond {
    fn eval(&self, row: &CsvRow) -> bool {
        let mut acc = self.first.eval(row);
        for (op, term) in &self.rest {
            let v = term.eval(row);
            acc = match op {
                LogicOp::And => acc && v,
                LogicOp::Or => acc || v,
            };
        }
        acc
    }
}

fn parse_expr(expr: &str) -> std::result::Result<Cond, String> {
    let chunks = split_logic(expr);
    if chunks.is_empty() {
        return Err("empty expression".to_string());
    }
    let first = parse_term(&chunks[0].1)?;
    let mut rest = Vec::new();
    for (op, text) in &chunks[1..] {
        let op = match op.as_str() {
            "and" => LogicOp::And,
            "or" => LogicOp::Or,
            other => return Err(format!("unknown logic operator {other:?}")),
        };
        rest.push((op, parse_term(text)?));
    }
    Ok(Cond { first, rest })
}

/// Split `a and b or c` into `[("", "a"), ("and", "b"), ("or", "c")]` on
/// whitespace-delimited `and`/`or` keywords, ignoring any that appear
/// inside a `/regex/` or `"string"` literal.
fn split_logic(expr: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut current_op = String::new();
    let mut in_regex = false;
    let mut in_string = false;
    let words: Vec<&str> = expr.split_whitespace().collect();
    for w in words {
        for c in w.chars() {
            if c == '/' && !in_string {
                in_regex = !in_regex;
            }
            if c == '"' && !in_regex {
                in_string = !in_string;
            }
        }
        if !in_regex && !in_string && (w == "and" || w == "or") {
            out.push((current_op.clone(), current.trim().to_string()));
            current_op = w.to_string();
            current.clear();
        } else {
            if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(w);
        }
    }
    out.push((current_op, current.trim().to_string()));
    out
}

fn parse_term(text: &str) -> std::result::Result<Term, String> {
    let text = text.trim();
    for (op_str, op) in [
        ("~", CondOp::Match),
        ("<=", CondOp::Le),
        (">=", CondOp::Ge),
        ("==", CondOp::Eq),
        ("!=", CondOp::Ne),
        ("<", CondOp::Lt),
        (">", CondOp::Gt),
    ] {
        if let Some(pos) = text.find(op_str) {
            let field = text[..pos].trim().to_string();
            let value_text = text[pos + op_str.len()..].trim();
            let value = parse_value(value_text, op)?;
            return Ok(Term { field, op, value });
        }
    }
    Err(format!("could not find a comparison operator in {text:?}"))
}

fn parse_value(text: &str, op: CondOp) -> std::result::Result<CondValue, String> {
    if op == CondOp::Match {
        let text = text
            .strip_prefix('/')
            .ok_or("regex value must start with /")?;
        let close = text
            .rfind('/')
            .ok_or("regex value must end with /[flags]")?;
        let pattern = &text[..close];
        let flags = &text[close + 1..];
        let alts: Vec<String> = pattern.split('|').map(|s| s.to_string()).collect();
        return Ok(CondValue::Regex(alts, flags.contains('i')));
    }
    if let Some(stripped) = text.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
        return Ok(CondValue::Str(stripped.to_string()));
    }
    Decimal::from_str(text)
        .map(CondValue::Number)
        .map_err(|e| format!("bad value {text:?}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(payee: &str, amount: &str) -> CsvRow {
        CsvRow {
            date: Date::from_ymd(2026, 1, 1).unwrap(),
            amount: Decimal::from_str(amount).unwrap(),
            payee: payee.to_string(),
            raw: BTreeMap::new(),
            dedupe_hash: "x".to_string(),
        }
    }

    #[test]
    fn matches_regex_alternation_case_insensitive() {
        let cond = parse_expr("payee ~ /uber|careem/i").unwrap();
        assert!(cond.eval(&row("UBER *TRIP", "-20")));
        assert!(cond.eval(&row("Careem ride", "-15")));
        assert!(!cond.eval(&row("Grocery store", "-15")));
    }

    #[test]
    fn matches_amount_and_regex_combo() {
        let cond = parse_expr("amount < 0 and payee ~ /rent/i").unwrap();
        assert!(cond.eval(&row("Monthly Rent", "-3000")));
        assert!(!cond.eval(&row("Monthly Rent", "3000")));
        assert!(!cond.eval(&row("Groceries", "-50")));
    }

    #[test]
    fn dedupe_hash_is_stable() {
        let d = Date::from_ymd(2026, 1, 1).unwrap();
        let a = Decimal::from_str("-12.50").unwrap();
        let h1 = dedupe_key(&d, &a, "Coffee", "bank.csv");
        let h2 = dedupe_key(&d, &a, "Coffee", "bank.csv");
        assert_eq!(h1, h2);
        let h3 = dedupe_key(&d, &a, "Coffee", "other.csv");
        assert_ne!(h1, h3);
    }
}

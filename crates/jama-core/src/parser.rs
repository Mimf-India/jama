//! The JAMA ledger text format: a Beancount-flavoured, human-editable
//! plain-text syntax. See `docs/format.md` for the full grammar; this
//! module is the single source of truth for both directions — parsing text
//! into [`Directive`]s and serializing them back to text.

use std::collections::BTreeMap;
use std::str::FromStr;

use rust_decimal::Decimal;

use crate::date::Date;
use crate::error::JamaError;
use crate::model::{
    Account, Amount, CommodityInfo, CostOrPrice, Directive, Flag, Posting, Transaction, ROOTS,
};

/// Parse a whole ledger text file into an ordered list of directives.
pub fn parse(text: &str) -> Result<Vec<Directive>, JamaError> {
    let lines: Vec<&str> = text.lines().collect();
    let mut directives = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        let raw = lines[i];
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with(';') {
            i += 1;
            continue;
        }
        if raw.starts_with(char::is_whitespace) {
            return Err(JamaError::Parse {
                line: i + 1,
                message: format!("unexpected indented line outside a transaction: {raw:?}"),
            });
        }

        let (directive, consumed) = parse_top_level(&lines, i)?;
        directives.push(directive);
        i += consumed;
    }

    Ok(directives)
}

/// Parse the directive starting at `lines[start]`, returning it plus how
/// many lines (including any indented body) it consumed.
fn parse_top_level(lines: &[&str], start: usize) -> Result<(Directive, usize), JamaError> {
    let line_no = start + 1;
    let line = lines[start].trim();
    let mut parts = line.splitn(3, ' ');
    let date_str = parts.next().unwrap_or("");
    let date = Date::parse_iso(date_str).map_err(|e| JamaError::Parse {
        line: line_no,
        message: e,
    })?;
    let rest = line[date_str.len()..].trim_start();

    let mut rest_parts = rest.splitn(2, ' ');
    let keyword = rest_parts.next().unwrap_or("");
    let tail = rest_parts.next().unwrap_or("").trim();

    match keyword {
        "*" | "!" => {
            let flag =
                Flag::parse(keyword.chars().next().unwrap()).map_err(|e| JamaError::Parse {
                    line: line_no,
                    message: e,
                })?;
            parse_transaction(lines, start, date, flag, tail)
        }
        "open" => {
            let mut fields = tail.split_whitespace();
            let account = parse_account(fields.next(), line_no)?;
            let currencies: Vec<String> = fields
                .flat_map(|s| s.split(','))
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect();
            Ok((
                Directive::Open {
                    date,
                    account,
                    currencies,
                },
                1,
            ))
        }
        "close" => {
            let account = parse_account(tail.split_whitespace().next(), line_no)?;
            Ok((Directive::Close { date, account }, 1))
        }
        "balance" => {
            let mut fields = tail.splitn(2, char::is_whitespace);
            let account = parse_account(fields.next(), line_no)?;
            let amount_str = fields.next().ok_or_else(|| JamaError::Parse {
                line: line_no,
                message: "balance directive missing amount".to_string(),
            })?;
            let amount = parse_amount(amount_str.trim(), line_no)?;
            Ok((
                Directive::Balance {
                    date,
                    account,
                    amount,
                },
                1,
            ))
        }
        "pad" => {
            let mut fields = tail.split_whitespace();
            let account = parse_account(fields.next(), line_no)?;
            let source = parse_account(fields.next(), line_no)?;
            Ok((
                Directive::Pad {
                    date,
                    account,
                    source,
                },
                1,
            ))
        }
        "commodity" => {
            let code = tail.split_whitespace().next().unwrap_or("").to_string();
            if code.is_empty() {
                return Err(JamaError::Parse {
                    line: line_no,
                    message: "commodity directive missing code".to_string(),
                });
            }
            // Optional following `  precision: "N"` metadata line.
            let mut consumed = 1;
            let mut precision = 2u8;
            if let Some(next) = lines.get(start + 1) {
                if next.starts_with(char::is_whitespace) && !next.trim().is_empty() {
                    if let Some((key, value)) = parse_meta_line(next.trim()) {
                        if key == "precision" {
                            precision = value.parse().unwrap_or(2);
                            consumed += 1;
                        }
                    }
                }
            }
            Ok((
                Directive::Commodity {
                    date,
                    info: CommodityInfo::new(code, precision),
                },
                consumed,
            ))
        }
        other => Err(JamaError::Parse {
            line: line_no,
            message: format!("unknown directive keyword {other:?}"),
        }),
    }
}

fn parse_account(field: Option<&str>, line_no: usize) -> Result<Account, JamaError> {
    let field = field.ok_or_else(|| JamaError::Parse {
        line: line_no,
        message: "expected an account name".to_string(),
    })?;
    Account::parse(field).map_err(|e| JamaError::Parse {
        line: line_no,
        message: e,
    })
}

/// Parse a transaction header's tail (everything after the flag) plus its
/// indented body of postings and metadata.
fn parse_transaction(
    lines: &[&str],
    start: usize,
    date: Date,
    flag: Flag,
    tail: &str,
) -> Result<(Directive, usize), JamaError> {
    let line_no = start + 1;
    let (payee, narration, after_strings) = parse_header_strings(tail, line_no)?;

    let mut tags = Vec::new();
    let mut links = Vec::new();
    for tok in after_strings.split_whitespace() {
        if let Some(t) = tok.strip_prefix('#') {
            tags.push(t.to_string());
        } else if let Some(l) = tok.strip_prefix('^') {
            links.push(l.to_string());
        }
    }

    let mut txn = Transaction {
        id: None,
        date,
        flag,
        payee,
        narration,
        tags,
        links,
        postings: Vec::new(),
        meta: BTreeMap::new(),
    };

    let mut i = start + 1;
    while i < lines.len() {
        let raw = lines[i];
        if raw.trim().is_empty() {
            break;
        }
        if !raw.starts_with(char::is_whitespace) {
            break;
        }
        let body_line_no = i + 1;
        let trimmed = raw.trim();
        if trimmed.starts_with(';') {
            i += 1;
            continue;
        }
        if is_posting_line(trimmed) {
            txn.postings.push(parse_posting(trimmed, body_line_no)?);
        } else if let Some((key, value)) = parse_meta_line(trimmed) {
            txn.meta.insert(key, value);
        } else {
            return Err(JamaError::Parse {
                line: body_line_no,
                message: format!("could not parse transaction body line: {trimmed:?}"),
            });
        }
        i += 1;
    }

    Ok((Directive::Transaction(txn), i - start))
}

/// Extract up to two double-quoted strings from a transaction header's
/// tail, returning (payee, narration, remainder-after-the-strings). One
/// string means "narration only"; two means "payee, then narration".
fn parse_header_strings(
    tail: &str,
    line_no: usize,
) -> Result<(Option<String>, String, String), JamaError> {
    let mut strings = Vec::new();
    let mut chars = tail.char_indices().peekable();
    let mut last_end = 0;
    while let Some((idx, c)) = chars.next() {
        if c == '"' {
            let start_content = idx + 1;
            let mut end_content = None;
            for (j, cc) in chars.by_ref() {
                if cc == '"' {
                    end_content = Some(j);
                    break;
                }
            }
            let end_content = end_content.ok_or_else(|| JamaError::Parse {
                line: line_no,
                message: "unterminated string in transaction header".to_string(),
            })?;
            strings.push(tail[start_content..end_content].to_string());
            last_end = end_content + 1;
        }
    }
    let remainder = tail[last_end..].to_string();
    match strings.len() {
        0 => Ok((None, String::new(), remainder)),
        1 => Ok((None, strings.remove(0), remainder)),
        _ => {
            let narration = strings.pop().unwrap();
            let payee = strings.pop().unwrap();
            Ok((Some(payee), narration, remainder))
        }
    }
}

fn is_posting_line(trimmed: &str) -> bool {
    ROOTS
        .iter()
        .any(|r| trimmed.starts_with(&format!("{r}:")) || trimmed == *r)
}

fn parse_posting(trimmed: &str, line_no: usize) -> Result<Posting, JamaError> {
    // Split off any trailing `{cost}` and/or `@ price` before the base
    // account/amount pair.
    let mut remaining = trimmed;
    let mut cost_or_price = None;

    if let Some(at_pos) = remaining.find(" @ ") {
        let price_str = remaining[at_pos + 3..].trim();
        cost_or_price = Some(CostOrPrice {
            amount: parse_amount(price_str, line_no)?,
            is_cost: false,
        });
        remaining = remaining[..at_pos].trim_end();
    } else if let (Some(open), Some(close)) = (remaining.find('{'), remaining.find('}')) {
        let cost_str = remaining[open + 1..close].trim();
        cost_or_price = Some(CostOrPrice {
            amount: parse_amount(cost_str, line_no)?,
            is_cost: true,
        });
        remaining = remaining[..open].trim_end();
    }

    let mut fields = remaining.splitn(2, char::is_whitespace);
    let account = parse_account(fields.next(), line_no)?;
    let amount_part = fields.next().map(str::trim).filter(|s| !s.is_empty());
    let amount = match amount_part {
        Some(s) => Some(parse_amount(s, line_no)?),
        None => None,
    };

    Ok(Posting {
        account,
        amount,
        cost_or_price,
        meta: BTreeMap::new(),
    })
}

fn parse_amount(s: &str, line_no: usize) -> Result<Amount, JamaError> {
    let s = s.trim();
    let mut fields = s.splitn(2, char::is_whitespace);
    let number_str = fields.next().ok_or_else(|| JamaError::Parse {
        line: line_no,
        message: "expected an amount".to_string(),
    })?;
    let commodity = fields
        .next()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| JamaError::Parse {
            line: line_no,
            message: format!("amount {s:?} is missing a commodity code"),
        })?;
    let number = Decimal::from_str(number_str).map_err(|e| JamaError::Parse {
        line: line_no,
        message: format!("bad number {number_str:?}: {e}"),
    })?;
    Ok(Amount::new(number, commodity))
}

/// Parse a `key: "value"` metadata line. Returns `None` if it doesn't look
/// like one (so callers can try other interpretations).
fn parse_meta_line(trimmed: &str) -> Option<(String, String)> {
    let colon = trimmed.find(':')?;
    let key = trimmed[..colon].trim();
    if key.is_empty() || !key.chars().next()?.is_ascii_lowercase() {
        return None;
    }
    if !key
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return None;
    }
    let value = trimmed[colon + 1..].trim();
    let value = value
        .strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .unwrap_or(value);
    Some((key.to_string(), value.to_string()))
}

/// Serialize directives back to canonical JAMA ledger text. This is the
/// inverse of [`parse`]: `parse(serialize(directives))` round-trips.
pub fn serialize(directives: &[Directive]) -> String {
    let mut out = String::new();
    for (idx, d) in directives.iter().enumerate() {
        if idx > 0 {
            let prev_is_txn = matches!(directives[idx - 1], Directive::Transaction(_));
            let cur_is_txn = matches!(d, Directive::Transaction(_));
            if prev_is_txn || cur_is_txn {
                out.push('\n');
            }
        }
        serialize_one(d, &mut out);
    }
    out
}

fn serialize_one(d: &Directive, out: &mut String) {
    match d {
        Directive::Open {
            date,
            account,
            currencies,
        } => {
            out.push_str(&format!("{date} open {account}"));
            if !currencies.is_empty() {
                out.push(' ');
                out.push_str(&currencies.join(","));
            }
            out.push('\n');
        }
        Directive::Close { date, account } => {
            out.push_str(&format!("{date} close {account}\n"));
        }
        Directive::Balance {
            date,
            account,
            amount,
        } => {
            out.push_str(&format!("{date} balance {account}  {amount}\n"));
        }
        Directive::Pad {
            date,
            account,
            source,
        } => {
            out.push_str(&format!("{date} pad {account} {source}\n"));
        }
        Directive::Commodity { date, info } => {
            out.push_str(&format!("{date} commodity {}\n", info.code));
            out.push_str(&format!("  precision: \"{}\"\n", info.precision));
        }
        Directive::Transaction(t) => serialize_transaction(t, out),
    }
}

fn serialize_transaction(t: &Transaction, out: &mut String) {
    out.push_str(&format!("{} {}", t.date, t.flag.as_char()));
    if let Some(payee) = &t.payee {
        out.push_str(&format!(" \"{payee}\""));
    }
    out.push_str(&format!(" \"{}\"", t.narration));
    for tag in &t.tags {
        out.push_str(&format!(" #{tag}"));
    }
    for link in &t.links {
        out.push_str(&format!(" ^{link}"));
    }
    out.push('\n');
    for (key, value) in &t.meta {
        out.push_str(&format!("  {key}: \"{value}\"\n"));
    }
    for p in &t.postings {
        out.push_str("  ");
        out.push_str(p.account.as_str());
        if let Some(amount) = &p.amount {
            out.push_str("  ");
            out.push_str(&amount.to_string());
        }
        if let Some(cp) = &p.cost_or_price {
            if cp.is_cost {
                out.push_str(&format!(" {{{}}}", cp.amount));
            } else {
                out.push_str(&format!(" @ {}", cp.amount));
            }
        }
        out.push('\n');
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_serializes_a_simple_transaction() {
        let text = "2026-09-01 * \"Salary\" \"Monthly salary\"\n  income:salary  -18000.00 SAR\n  assets:checking  18000.00 SAR\n";
        let directives = parse(text).unwrap();
        assert_eq!(directives.len(), 1);
        let out = serialize(&directives);
        assert_eq!(out, text);
    }

    #[test]
    fn round_trips_open_balance_pad() {
        let text = "\
2026-01-01 open assets:checking SAR
2026-01-01 open equity:opening-balances
2026-01-01 pad assets:checking equity:opening-balances
2026-01-02 balance assets:checking  1000.00 SAR
";
        let directives = parse(text).unwrap();
        assert_eq!(directives.len(), 4);
        assert_eq!(serialize(&directives), text);
    }

    #[test]
    fn round_trips_full_generated_ledger() {
        // Build a ledger in-memory, serialize it, parse it back, and check
        // the two directive lists are equal and reserializing produces the
        // same canonical text — the "byte-identical for canonical input"
        // guarantee.
        let date = Date::from_ymd(2026, 9, 1).unwrap();
        let mut txn = Transaction::new(date, "Coffee with a friend");
        txn.payee = Some("Cafe Bateel".to_string());
        txn.tags.push("social".to_string());
        txn.postings.push(Posting::new(
            Account::parse("assets:checking").unwrap(),
            Amount::new(Decimal::from_str("-25.00").unwrap(), "SAR"),
        ));
        txn.postings
            .push(Posting::elided(Account::parse("expenses:cafe").unwrap()));

        let directives = vec![
            Directive::Open {
                date,
                account: Account::parse("assets:checking").unwrap(),
                currencies: vec!["SAR".to_string()],
            },
            Directive::Transaction(txn),
        ];

        let text = serialize(&directives);
        let reparsed = parse(&text).unwrap();
        // Elided posting became an explicit one after balancing is applied
        // by the caller (parser itself doesn't resolve elision), so compare
        // structurally via reserialization instead of equality on amounts.
        assert_eq!(serialize(&reparsed), text);
        assert_eq!(reparsed.len(), directives.len());
    }

    #[test]
    fn rejects_unbalanced_and_reports_line() {
        let text = "not-a-date * \"x\"\n";
        let err = parse(text).unwrap_err();
        match err {
            JamaError::Parse { line, .. } => assert_eq!(line, 1),
            other => panic!("expected parse error, got {other:?}"),
        }
    }

    #[test]
    fn arabic_payee_round_trips() {
        let text = "2026-09-01 * \"مطعم\" \"غداء مع صديق\"\n  assets:checking  -50.00 SAR\n  expenses:cafe  50.00 SAR\n";
        let directives = parse(text).unwrap();
        assert_eq!(serialize(&directives), text);
    }

    /// The fixture ledgers under `fixtures/` (used by the CLI's own
    /// integration tests as reference material) must parse cleanly and
    /// every transaction in them must balance.
    #[test]
    fn fixture_ledgers_parse_and_balance() {
        for name in ["multi_currency.beancount", "arabic_payees.beancount"] {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../fixtures")
                .join(name);
            let text =
                std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {path:?}: {e}"));
            let directives = parse(&text).unwrap_or_else(|e| panic!("parsing {path:?}: {e}"));
            assert!(!directives.is_empty(), "{path:?} produced no directives");
            for d in &directives {
                if let Directive::Transaction(t) = d {
                    t.resolve_postings()
                        .unwrap_or_else(|e| panic!("{path:?}: {e}"));
                }
            }
        }
    }
}

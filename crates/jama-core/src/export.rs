//! Rendering a [`Store`]'s contents to a portable text format. Beancount is
//! the primary, round-trippable one; `ledger` and `csv` are one-way exports
//! for interoperating with other tools.

use crate::error::Result;
use crate::model::Directive;
use crate::parser;
use crate::store::Store;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    Beancount,
    Ledger,
    Csv,
}

impl ExportFormat {
    pub fn parse(s: &str) -> std::result::Result<Self, String> {
        match s.to_lowercase().as_str() {
            "beancount" | "bean" => Ok(ExportFormat::Beancount),
            "ledger" | "hledger" => Ok(ExportFormat::Ledger),
            "csv" => Ok(ExportFormat::Csv),
            other => Err(format!(
                "unknown export format {other:?} (expected beancount, ledger, or csv)"
            )),
        }
    }
}

/// Collect every directive currently in the store, in ledger order: account
/// opens/closes and commodity declarations first (sorted by date, then
/// name), then transactions and balance assertions interleaved by date.
pub fn all_directives(store: &Store) -> Result<Vec<Directive>> {
    let mut directives = Vec::new();

    for info in store.list_commodities()? {
        directives.push(Directive::Commodity {
            date: crate::date::Date::from_ymd(1970, 1, 1).unwrap(),
            info,
        });
    }
    for (account, opened, closed, currencies) in store.list_account_declarations()? {
        directives.push(Directive::Open {
            date: opened,
            account: account.clone(),
            currencies,
        });
        if let Some(closed_on) = closed {
            directives.push(Directive::Close {
                date: closed_on,
                account,
            });
        }
    }
    for (date, account, source) in store.list_pads()? {
        directives.push(Directive::Pad {
            date,
            account,
            source,
        });
    }
    for (date, account, amount) in store.list_balance_assertions()? {
        directives.push(Directive::Balance {
            date,
            account,
            amount,
        });
    }
    for txn in store.all_transactions()? {
        directives.push(Directive::Transaction(txn));
    }

    directives.sort_by_key(|d| (d.date(), directive_rank(d)));
    Ok(directives)
}

fn directive_rank(d: &Directive) -> u8 {
    match d {
        Directive::Commodity { .. } => 0,
        Directive::Open { .. } => 1,
        Directive::Pad { .. } => 2,
        Directive::Balance { .. } => 3,
        Directive::Transaction(_) => 4,
        Directive::Close { .. } => 5,
    }
}

pub fn render(store: &Store, format: ExportFormat) -> Result<String> {
    let directives = all_directives(store)?;
    Ok(match format {
        ExportFormat::Beancount => parser::serialize(&directives),
        ExportFormat::Ledger => render_ledger(&directives),
        ExportFormat::Csv => render_csv(&directives),
    })
}

/// Render in ledger-cli's own (similar, but not identical) text syntax.
fn render_ledger(directives: &[Directive]) -> String {
    let mut out = String::new();
    for d in directives {
        match d {
            Directive::Transaction(t) => {
                let payee_and_narration = match &t.payee {
                    Some(p) => format!("{p} - {}", t.narration),
                    None => t.narration.clone(),
                };
                out.push_str(&format!("{} {}\n", t.date, payee_and_narration));
                for p in &t.postings {
                    let amount = p
                        .amount
                        .as_ref()
                        .map(|a| format!("{} {}", a.number, a.commodity))
                        .unwrap_or_default();
                    out.push_str(&format!("    {:<40}{}\n", p.account.as_str(), amount));
                }
                out.push('\n');
            }
            Directive::Open {
                date,
                account,
                currencies,
            } => {
                out.push_str(&format!("{date} open {account}"));
                if !currencies.is_empty() {
                    out.push_str(&format!(" {}", currencies.join(",")));
                }
                out.push('\n');
            }
            Directive::Close { date, account } => {
                out.push_str(&format!("{date} close {account}\n"))
            }
            Directive::Balance {
                date,
                account,
                amount,
            } => out.push_str(&format!("{date} balance {account}  {amount}\n")),
            Directive::Pad {
                date,
                account,
                source,
            } => out.push_str(&format!("{date} pad {account} {source}\n")),
            Directive::Commodity { .. } => {}
        }
    }
    out
}

/// Flatten transactions to one row per posting — the common shape other
/// tools (and spreadsheets) expect from a "CSV export".
fn render_csv(directives: &[Directive]) -> String {
    let mut out = String::from("date,flag,payee,narration,account,amount,commodity\n");
    for d in directives {
        if let Directive::Transaction(t) = d {
            for p in &t.postings {
                let amount = p.amount.as_ref();
                out.push_str(&format!(
                    "{},{},{},{},{},{},{}\n",
                    t.date,
                    t.flag.as_char(),
                    csv_escape(t.payee.as_deref().unwrap_or("")),
                    csv_escape(&t.narration),
                    p.account.as_str(),
                    amount.map(|a| a.number.to_string()).unwrap_or_default(),
                    amount.map(|a| a.commodity.clone()).unwrap_or_default(),
                ));
            }
        }
    }
    out
}

fn csv_escape(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

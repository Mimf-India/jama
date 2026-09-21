use jama_core::reports;
use jama_core::Ledger;

use crate::color::Palette;
use crate::render::{flag_badge, format_amount, Align, Table};
use crate::{Cli, CliError};

pub fn run(
    ledger: &Ledger,
    cli: &Cli,
    palette: &Palette,
    pattern: &str,
    since: &Option<String>,
    until: &Option<String>,
) -> Result<(), CliError> {
    let since = super::parse_date_flag(since, "--since")?;
    let until = super::parse_date_flag(until, "--until")?;
    let entries = reports::register(&ledger.store, pattern, since, until)?;

    if cli.json {
        let items: Vec<serde_json::Value> = entries
            .iter()
            .map(|e| {
                serde_json::json!({
                    "transaction_id": e.transaction_id,
                    "date": e.date.to_string(),
                    "flag": e.flag.as_char().to_string(),
                    "payee": e.payee,
                    "narration": e.narration,
                    "account": e.account,
                    "amount": crate::json::amount_json(&e.amount),
                    "running_balance": e.running_balance.iter().map(crate::json::amount_json).collect::<Vec<_>>(),
                })
            })
            .collect();
        super::print_json(&serde_json::json!({ "entries": items }));
        return Ok(());
    }

    if entries.is_empty() {
        if !cli.quiet {
            println!("no postings match {pattern}");
        }
        return Ok(());
    }

    let mut table = Table::new(
        vec!["Date", "", "Description", "Account", "Amount", "Balance"],
        vec![
            Align::Left,
            Align::Left,
            Align::Left,
            Align::Left,
            Align::Right,
            Align::Right,
        ],
    );
    for e in &entries {
        let desc = match &e.payee {
            Some(p) => format!("{p} — {}", e.narration),
            None => e.narration.clone(),
        };
        let precision = ledger
            .store
            .commodity_precision(&e.amount.commodity)
            .unwrap_or(2);
        let running: Vec<String> = e
            .running_balance
            .iter()
            .map(|a| {
                format_amount(
                    a,
                    ledger.store.commodity_precision(&a.commodity).unwrap_or(2),
                    palette,
                )
            })
            .collect();
        table.push_row(vec![
            e.date.to_string(),
            flag_badge(e.flag, palette),
            desc,
            e.account.clone(),
            format_amount(&e.amount, precision, palette),
            running.join(", "),
        ]);
    }
    print!("{}", table.render());
    Ok(())
}

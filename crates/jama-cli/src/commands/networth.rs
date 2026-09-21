use jama_core::reports;
use jama_core::Ledger;

use crate::color::Palette;
use crate::render::{format_amount, Align, Table};
use crate::{Cli, CliError};

pub fn run(
    ledger: &Ledger,
    cli: &Cli,
    palette: &Palette,
    since: &Option<String>,
    until: &Option<String>,
    monthly: bool,
) -> Result<(), CliError> {
    let since = super::parse_date_flag(since, "--since")?;
    let until = super::parse_date_flag(until, "--until")?;
    let points = reports::networth(&ledger.store, since, until, monthly)?;

    if cli.json {
        let items: Vec<serde_json::Value> = points
            .iter()
            .map(|p| {
                serde_json::json!({
                    "as_of": p.as_of.to_string(),
                    "balances": p.balances.iter().map(crate::json::amount_json).collect::<Vec<_>>(),
                })
            })
            .collect();
        super::print_json(&serde_json::json!({ "networth": items }));
        return Ok(());
    }

    if points.is_empty() {
        if !cli.quiet {
            println!("no transactions to compute net worth from");
        }
        return Ok(());
    }

    if monthly {
        let mut table = Table::new(vec!["Month", "Net worth"], vec![Align::Left, Align::Right]);
        for p in &points {
            let rendered: Vec<String> = p
                .balances
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
                format!("{}-{:02}", p.as_of.year(), p.as_of.month()),
                rendered.join(", "),
            ]);
        }
        print!("{}", table.render());
    } else {
        let p = &points[0];
        let rendered: Vec<String> = p
            .balances
            .iter()
            .map(|a| {
                format_amount(
                    a,
                    ledger.store.commodity_precision(&a.commodity).unwrap_or(2),
                    palette,
                )
            })
            .collect();
        println!("net worth as of {}: {}", p.as_of, rendered.join(", "));
    }
    Ok(())
}

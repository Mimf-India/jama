use jama_core::reports::{self, BalanceOptions};
use jama_core::Ledger;
use rust_decimal::prelude::ToPrimitive;

use crate::color::Palette;
use crate::render::{format_amount, Align, Table};
use crate::{Cli, CliError};

#[allow(clippy::too_many_arguments)]
pub fn run(
    ledger: &Ledger,
    cli: &Cli,
    palette: &Palette,
    pattern: Option<&str>,
    depth: Option<usize>,
    since: &Option<String>,
    until: &Option<String>,
    cleared: bool,
) -> Result<(), CliError> {
    let opts = BalanceOptions {
        account_pattern: pattern.map(str::to_string),
        depth,
        since: super::parse_date_flag(since, "--since")?,
        until: super::parse_date_flag(until, "--until")?,
        cleared_only: cleared,
    };
    let balances = reports::balance(&ledger.store, &opts)?;

    if cli.json {
        let accounts: Vec<serde_json::Value> = balances
            .iter()
            .map(|ab| {
                let balances_json: Vec<serde_json::Value> =
                    ab.balances.iter().map(crate::json::amount_json).collect();
                let primary = ab.balances.first().and_then(|a| a.number.to_f64());
                serde_json::json!({
                    "account": ab.account,
                    "balances": balances_json,
                    "balance": primary,
                })
            })
            .collect();
        super::print_json(&serde_json::json!({ "accounts": accounts }));
        return Ok(());
    }

    if balances.is_empty() {
        if !cli.quiet {
            println!("no balances to show");
        }
        return Ok(());
    }

    let mut table = Table::new(vec!["Account", "Balance"], vec![Align::Left, Align::Right]);
    for ab in &balances {
        let rendered: Vec<String> = ab
            .balances
            .iter()
            .map(|a| {
                let precision = ledger.store.commodity_precision(&a.commodity).unwrap_or(2);
                format_amount(a, precision, palette)
            })
            .collect();
        table.push_row(vec![ab.account.clone(), rendered.join(", ")]);
    }
    print!("{}", table.render());
    Ok(())
}

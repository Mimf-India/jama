use jama_core::model::Directive;
use jama_core::store::ListFilter;
use jama_core::{parser, Ledger};

use crate::color::Palette;
use crate::{Cli, CliError};

#[allow(clippy::too_many_arguments)]
pub fn run(
    ledger: &Ledger,
    cli: &Cli,
    palette: &Palette,
    since: &Option<String>,
    until: &Option<String>,
    account: &Option<String>,
    payee: &Option<String>,
    tag: &Option<String>,
    limit: Option<u32>,
) -> Result<(), CliError> {
    let _ = palette;
    let filter = ListFilter {
        since: super::parse_date_flag(since, "--since")?,
        until: super::parse_date_flag(until, "--until")?,
        account: account.clone(),
        payee: payee.clone(),
        tag: tag.clone(),
        limit,
    };
    let txns = ledger.list_transactions(&filter)?;

    if cli.json {
        let items: Vec<serde_json::Value> =
            txns.iter().map(crate::json::transaction_json).collect();
        super::print_json(&serde_json::json!({ "transactions": items }));
        return Ok(());
    }

    if txns.is_empty() {
        if !cli.quiet {
            println!("no transactions match");
        }
        return Ok(());
    }

    for t in &txns {
        println!("; id: {}", t.id.unwrap_or_default());
        print!(
            "{}",
            parser::serialize(std::slice::from_ref(&Directive::Transaction(t.clone())))
        );
        println!();
    }
    Ok(())
}

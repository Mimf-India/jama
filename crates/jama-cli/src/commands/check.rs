use jama_core::reports;
use jama_core::Ledger;

use crate::color::{Palette, Tone};
use crate::{Cli, CliError};

pub fn run(ledger: &Ledger, cli: &Cli, palette: &Palette) -> Result<(), CliError> {
    let report = reports::check(&ledger.store)?;

    if cli.json {
        super::print_json(&serde_json::json!({
            "transactions_checked": report.transactions_checked,
            "imbalances": report.imbalances,
            "failed_assertions": report.failed_assertions,
            "undeclared_accounts": report.undeclared_accounts,
            "clean": report.is_clean(),
        }));
    } else if !cli.quiet {
        let mark = if report.is_clean() {
            palette.paint(Tone::Teal, "✓")
        } else {
            palette.paint(Tone::Signal, "✗")
        };
        let mut parts = vec![
            format!("{} transactions", format_count(report.transactions_checked)),
            format!("{} imbalances", report.imbalances.len()),
        ];
        if !report.undeclared_accounts.is_empty() {
            parts.push(format!(
                "{} undeclared accounts (warning)",
                report.undeclared_accounts.len()
            ));
        }
        if !report.failed_assertions.is_empty() {
            parts.push(format!(
                "{} failed balance assertions",
                report.failed_assertions.len()
            ));
        }
        println!("{mark} {}", parts.join(" · "));
        for line in report
            .imbalances
            .iter()
            .chain(report.failed_assertions.iter())
        {
            println!("  - {line}");
        }
    }

    if !report.is_clean() {
        std::process::exit(2);
    }
    Ok(())
}

fn format_count(n: usize) -> String {
    crate::render::format_number(rust_decimal::Decimal::from(n), 0)
}

use std::io::{self, Write};
use std::path::Path;

use jama_core::csv_import::{self, Rules};
use jama_core::model::Account;
use jama_core::Ledger;

use crate::color::Palette;
use crate::{Cli, CliError};

pub fn run(
    ledger: &mut Ledger,
    cli: &Cli,
    palette: &Palette,
    file: &Path,
    rules_path: Option<&Path>,
    dry_run: bool,
) -> Result<(), CliError> {
    let _ = palette;
    let rules_path = match rules_path {
        Some(p) => p.to_path_buf(),
        None => {
            let stem = file
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("import");
            ledger.paths.rules_dir.join(format!("{stem}.toml"))
        }
    };
    if !rules_path.exists() {
        return Err(CliError::user(
            format!("no rules file at {}", rules_path.display()),
            "create it (see docs/format.md for the [source]/[[rule]] shape) or pass --rules explicitly",
        ));
    }
    let rules = Rules::load(&rules_path)?;
    let mut outcome = csv_import::import(ledger, file, &rules, dry_run)?;

    let mut categorized = 0usize;
    if !dry_run && !outcome.unmatched.is_empty() && !cli.quiet && !cli.json {
        let source_account = Account::parse(&rules.source.account).map_err(CliError::internal)?;
        let mut still_unmatched = Vec::new();
        for row in outcome.unmatched.drain(..) {
            println!(
                "unmatched: {} {} {} — {}",
                row.date,
                row.amount,
                row.raw
                    .get(&rules.source.payee)
                    .map(String::as_str)
                    .unwrap_or(""),
                row.payee
            );
            let dest = prompt("  categorise as (blank to skip): ")?;
            let dest = dest.trim();
            if dest.is_empty() {
                still_unmatched.push(row);
                continue;
            }
            csv_import::insert_row(ledger, &source_account, dest, &row)?;
            let _ = Rules::append_learned_rule(&rules_path, &row.payee, dest);
            categorized += 1;
        }
        outcome.unmatched = still_unmatched;
        ledger.sync_text_file()?;
    }

    if cli.json {
        crate::commands::print_json(&serde_json::json!({
            "total_rows": outcome.total_rows,
            "imported": outcome.imported,
            "skipped_duplicates": outcome.skipped_duplicates,
            "categorized_interactively": categorized,
            "unmatched": outcome.unmatched.len(),
            "dry_run": dry_run,
        }));
    } else if !cli.quiet {
        let mut summary = format!(
            "{} rows · {} imported · {} skipped (duplicates)",
            outcome.total_rows, outcome.imported, outcome.skipped_duplicates
        );
        if categorized > 0 {
            summary.push_str(&format!(" · {categorized} categorised interactively"));
        }
        if !outcome.unmatched.is_empty() {
            summary.push_str(&format!(" · {} unmatched", outcome.unmatched.len()));
        }
        if dry_run {
            summary.push_str(" (dry run — nothing written)");
        }
        println!("{summary}");
    }
    Ok(())
}

fn prompt(label: &str) -> Result<String, CliError> {
    print!("{label}");
    io::stdout()
        .flush()
        .map_err(|e| CliError::internal(e.to_string()))?;
    let mut line = String::new();
    io::stdin()
        .read_line(&mut line)
        .map_err(|e| CliError::internal(e.to_string()))?;
    Ok(line)
}

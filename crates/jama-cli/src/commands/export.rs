use std::path::Path;

use jama_core::export::ExportFormat;
use jama_core::Ledger;

use crate::{Cli, CliError};

pub fn run(ledger: &Ledger, cli: &Cli, format: &str, out: Option<&Path>) -> Result<(), CliError> {
    let format = ExportFormat::parse(format)
        .map_err(|e| CliError::user(e, "use beancount, ledger, or csv"))?;
    let text = jama_core::export::render(&ledger.store, format)?;

    match out {
        Some(path) => {
            std::fs::write(path, &text)
                .map_err(|e| CliError::internal(format!("writing {}: {e}", path.display())))?;
            if !cli.quiet {
                if cli.json {
                    crate::commands::print_json(
                        &serde_json::json!({ "written_to": path.display().to_string() }),
                    );
                } else {
                    println!("exported to {}", path.display());
                }
            }
        }
        None => print!("{text}"),
    }
    Ok(())
}

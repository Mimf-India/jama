use jama_core::Ledger;

use crate::color::Palette;
use crate::{Cli, CliError};

pub fn run(ledger: &mut Ledger, cli: &Cli, palette: &Palette, id: i64) -> Result<(), CliError> {
    let _ = palette;
    let removed = ledger.remove_transaction(id)?;
    if !removed {
        return Err(CliError::user(
            format!("no transaction #{id}"),
            "run `jama list` to see valid ids",
        ));
    }
    if !cli.quiet {
        if cli.json {
            crate::commands::print_json(&serde_json::json!({ "id": id, "removed": true }));
        } else {
            println!("removed transaction #{id}");
        }
    }
    Ok(())
}

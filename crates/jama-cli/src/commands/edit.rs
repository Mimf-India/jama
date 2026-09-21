use std::process::Command;

use jama_core::model::Directive;
use jama_core::{parser, Ledger};

use crate::{Cli, CliError};

pub fn run(ledger: &mut Ledger, cli: &Cli, id: i64) -> Result<(), CliError> {
    let txn = ledger.get_transaction(id)?.ok_or_else(|| {
        CliError::user(
            format!("no transaction #{id}"),
            "run `jama list` to see valid ids",
        )
    })?;

    let text = format!("; editing transaction #{id} — save and close to apply, or leave it unbalanced to abort\n{}", parser::serialize(&[Directive::Transaction(txn)]));

    let tmp = std::env::temp_dir().join(format!("jama-edit-{id}.beancount"));
    std::fs::write(&tmp, &text)
        .map_err(|e| CliError::internal(format!("writing temp file: {e}")))?;

    let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vi".to_string());
    let mut parts = editor.split_whitespace();
    let program = parts.next().unwrap_or("vi");
    let status = Command::new(program)
        .args(parts)
        .arg(&tmp)
        .status()
        .map_err(|e| {
            CliError::user(
                format!("could not launch $EDITOR ({editor}): {e}"),
                "set $EDITOR to a valid command",
            )
        })?;
    if !status.success() {
        return Err(CliError::user(
            "editor exited with an error",
            "re-run `jama edit` and save the file",
        ));
    }

    let edited = std::fs::read_to_string(&tmp)
        .map_err(|e| CliError::internal(format!("reading temp file: {e}")))?;
    let _ = std::fs::remove_file(&tmp);

    let directives = parser::parse(&edited)?;
    let txn = directives
        .into_iter()
        .find_map(|d| match d {
            Directive::Transaction(t) => Some(t),
            _ => None,
        })
        .ok_or_else(|| {
            CliError::user(
                "no transaction found in edited file",
                "keep the `DATE FLAG \"...\"` header and its postings",
            )
        })?;

    ledger.update_transaction(id, txn)?;

    if !cli.quiet {
        if cli.json {
            crate::commands::print_json(&serde_json::json!({ "id": id, "updated": true }));
        } else {
            println!("updated transaction #{id}");
        }
    }
    Ok(())
}

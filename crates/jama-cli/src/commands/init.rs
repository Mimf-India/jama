use std::path::Path;

use jama_core::Ledger;

use crate::color::{Palette, Tone, BRAND_MARK};
use crate::{Cli, CliError};

pub fn run(
    root: &Path,
    cli: &Cli,
    palette: &Palette,
    commodity: Option<&str>,
) -> Result<(), CliError> {
    let ledger = Ledger::init(root)?;
    if let Some(code) = commodity {
        ledger.set_default_commodity(code)?;
    }

    if !cli.quiet {
        if cli.json {
            crate::commands::print_json(&serde_json::json!({
                "root": root.display().to_string(),
                "db": root.join("jama.db").display().to_string(),
                "ledger": root.join("jama.beancount").display().to_string(),
                "rules_dir": root.join("rules").display().to_string(),
                "default_commodity": commodity,
            }));
        } else {
            let mark = palette.paint(Tone::Brand, &format!(" {BRAND_MARK} "));
            println!("{mark} JAMA v{}", env!("CARGO_PKG_VERSION"));
            println!();
            println!("Ledger created at {}", root.display());
            println!(
                "  {}  — the SQLite working store",
                root.join("jama.db").display()
            );
            println!(
                "  {}  — the source-of-truth text ledger",
                root.join("jama.beancount").display()
            );
            println!("  {}  — CSV import rules", root.join("rules").display());
            if let Some(code) = commodity {
                println!("  default commodity: {code}");
            }
            println!();
            println!("Next: jama add \"Opening balance\" 0 --from equity:opening-balances --to assets:checking{}", if commodity.is_some() { "" } else { " --commodity CODE" });
        }
    }
    Ok(())
}

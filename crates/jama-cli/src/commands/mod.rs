mod add;
mod balance;
mod check;
mod completions;
mod edit;
mod export;
mod import;
mod init;
mod list;
mod networth;
mod register;
mod rm;

use jama_core::{Date, Ledger};

use crate::color::Palette;
use crate::{Cli, CliError, Commands};

pub fn dispatch(cli: &Cli, command: &Commands, palette: &Palette) -> Result<(), CliError> {
    let root = jama_core::ledger::Paths::resolve(cli.file.as_deref());

    if let Commands::Init { path, commodity } = command {
        let root = path.clone().unwrap_or(root);
        return init::run(&root, cli, palette, commodity.as_deref());
    }
    if let Commands::Completions { shell } = command {
        return completions::run(*shell);
    }

    let mut ledger = Ledger::open(&root)?.with_strict(cli.strict);

    match command {
        Commands::Init { .. } => unreachable!(),
        Commands::Add {
            narration,
            amount,
            from,
            to,
            date,
            commodity,
            payee,
        } => add::run(
            &mut ledger,
            cli,
            palette,
            narration,
            amount,
            from,
            to,
            date,
            commodity,
            payee,
        ),
        Commands::Edit { id } => edit::run(&mut ledger, cli, *id),
        Commands::Rm { id } => rm::run(&mut ledger, cli, palette, *id),
        Commands::List {
            since,
            until,
            account,
            payee,
            tag,
            limit,
        } => list::run(
            &ledger, cli, palette, since, until, account, payee, tag, *limit,
        ),
        Commands::Balance {
            pattern,
            depth,
            since,
            until,
            cleared,
        } => balance::run(
            &ledger,
            cli,
            palette,
            pattern.as_deref(),
            *depth,
            since,
            until,
            *cleared,
        ),
        Commands::Register {
            pattern,
            since,
            until,
        } => register::run(&ledger, cli, palette, pattern, since, until),
        Commands::Networth {
            since,
            until,
            monthly,
        } => networth::run(&ledger, cli, palette, since, until, *monthly),
        Commands::Import {
            csv_file,
            rules,
            dry_run,
        } => import::run(
            &mut ledger,
            cli,
            palette,
            csv_file,
            rules.as_deref(),
            *dry_run,
        ),
        Commands::Export { format, out } => export::run(&ledger, cli, format, out.as_deref()),
        Commands::Check => check::run(&ledger, cli, palette),
        Commands::Completions { .. } => unreachable!(),
    }
}

/// Parse a `--since`/`--until`-style date flag, accepting `today`,
/// `yesterday`, and `YYYY-MM-DD`.
pub fn parse_date_flag(s: &Option<String>, flag: &str) -> Result<Option<Date>, CliError> {
    match s {
        None => Ok(None),
        Some(text) => Date::parse(text).map(Some).map_err(|e| {
            CliError::user(
                format!("bad {flag} value {text:?}: {e}"),
                "use YYYY-MM-DD, `today`, or `yesterday`",
            )
        }),
    }
}

pub fn print_json(value: &serde_json::Value) {
    println!(
        "{}",
        serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_string())
    );
}

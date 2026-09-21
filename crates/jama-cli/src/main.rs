mod color;
mod commands;
mod json;
mod render;

use std::io::Write;
use std::path::PathBuf;

use clap::{Parser, Subcommand};
use jama_core::JamaError;

use color::{Palette, Tone, BRAND_MARK};

#[derive(Parser)]
#[command(
    name = "jama",
    bin_name = "jama",
    about = "Plain-text accounting as one self-contained binary",
    disable_version_flag = true
)]
pub struct Cli {
    /// Ledger directory (else $JAMA_HOME, else ./jama, else ~/.jama).
    #[arg(short = 'f', long = "file", global = true, value_name = "LEDGER_DIR")]
    pub file: Option<PathBuf>,

    /// Emit machine-readable JSON instead of formatted text.
    #[arg(long, global = true)]
    pub json: bool,

    /// Disable coloured output.
    #[arg(long = "no-color", global = true)]
    pub no_color: bool,

    /// Suppress non-essential output.
    #[arg(short = 'q', long, global = true)]
    pub quiet: bool,

    /// Reject postings to undeclared accounts instead of warning.
    #[arg(long, global = true)]
    pub strict: bool,

    /// Print the version (and the JAMA mark) and exit.
    #[arg(short = 'V', long = "version")]
    pub version: bool,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Create a new ledger directory.
    Init { path: Option<PathBuf> },
    /// Add a transaction (interactively, or fully from flags).
    Add {
        narration: Option<String>,
        amount: Option<String>,
        #[arg(long)]
        from: Option<String>,
        #[arg(long)]
        to: Option<String>,
        #[arg(long)]
        date: Option<String>,
        #[arg(long, default_value = "SAR")]
        commodity: String,
        #[arg(long)]
        payee: Option<String>,
    },
    /// Open a transaction in $EDITOR as text, re-parsing it on save.
    Edit { id: i64 },
    /// Remove a transaction by id.
    Rm { id: i64 },
    /// List transactions, optionally filtered.
    List {
        #[arg(long)]
        since: Option<String>,
        #[arg(long)]
        until: Option<String>,
        #[arg(long)]
        account: Option<String>,
        #[arg(long)]
        payee: Option<String>,
        #[arg(long)]
        tag: Option<String>,
        #[arg(long)]
        limit: Option<u32>,
    },
    /// Show account balances.
    Balance {
        pattern: Option<String>,
        #[arg(long)]
        depth: Option<usize>,
        #[arg(long)]
        since: Option<String>,
        #[arg(long)]
        until: Option<String>,
        #[arg(long)]
        cleared: bool,
    },
    /// Show the transaction history and running balance of one account.
    Register {
        pattern: String,
        #[arg(long)]
        since: Option<String>,
        #[arg(long)]
        until: Option<String>,
    },
    /// Show net worth (assets minus liabilities) over time.
    Networth {
        #[arg(long)]
        since: Option<String>,
        #[arg(long)]
        until: Option<String>,
        #[arg(long)]
        monthly: bool,
    },
    /// Import a bank CSV using a rules file.
    Import {
        /// The bank CSV file to import.
        csv_file: PathBuf,
        #[arg(long)]
        rules: Option<PathBuf>,
        #[arg(long = "dry-run")]
        dry_run: bool,
    },
    /// Export the ledger to a portable text format.
    Export {
        #[arg(long = "format", default_value = "beancount")]
        format: String,
        #[arg(long = "out")]
        out: Option<PathBuf>,
    },
    /// Validate the ledger: balances, assertions, unknown accounts.
    Check,
    /// Generate shell completions.
    Completions { shell: clap_complete::Shell },
}

/// Piping `jama`'s output into `head` or `less` and letting the reader
/// close early should exit quietly, not panic with a broken-pipe
/// backtrace — restore the default SIGPIPE behaviour Rust disables by
/// default.
#[cfg(unix)]
fn restore_default_sigpipe() {
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
}

#[cfg(not(unix))]
fn restore_default_sigpipe() {}

fn main() {
    restore_default_sigpipe();
    let cli = Cli::parse();
    let palette = Palette::detect(cli.no_color);

    if cli.version {
        print_splash(&palette);
        return;
    }

    let Some(command) = cli.command.as_ref() else {
        eprintln!("no command given — try `jama --help`");
        std::process::exit(1);
    };

    match commands::dispatch(&cli, command, &palette) {
        Ok(()) => {}
        Err(e) => {
            print_error(&e);
            std::process::exit(exit_code_for(&e));
        }
    }
}

fn print_splash(palette: &Palette) {
    if palette.enabled() {
        let mark = palette.paint(Tone::Brand, &format!(" {BRAND_MARK} "));
        println!("{mark} JAMA v{}", env!("CARGO_PKG_VERSION"));
    } else {
        // No colour available (NO_COLOR, --no-color, or non-TTY stdout):
        // keep the mark legible with plain brackets instead of a colour chip.
        println!("[{BRAND_MARK}] JAMA v{}", env!("CARGO_PKG_VERSION"));
    }
}

/// Errors: one line of what happened, one line of where, one line of the
/// fix — never a raw Rust error dump.
fn print_error(e: &CliError) {
    let mut stderr = std::io::stderr();
    let _ = writeln!(stderr, "error: {}", e.what);
    if let Some(where_) = &e.where_ {
        let _ = writeln!(stderr, "  at: {where_}");
    }
    if let Some(fix) = &e.fix {
        let _ = writeln!(stderr, "  fix: {fix}");
    }
}

fn exit_code_for(e: &CliError) -> i32 {
    e.exit_code
}

/// A CLI-facing error: what happened, where, and how to fix it, plus the
/// exit code it should produce.
pub struct CliError {
    pub what: String,
    pub where_: Option<String>,
    pub fix: Option<String>,
    pub exit_code: i32,
}

impl CliError {
    pub fn user(what: impl Into<String>, fix: impl Into<String>) -> Self {
        Self {
            what: what.into(),
            where_: None,
            fix: Some(fix.into()),
            exit_code: 1,
        }
    }

    pub fn internal(what: impl Into<String>) -> Self {
        Self {
            what: what.into(),
            where_: None,
            fix: None,
            exit_code: 3,
        }
    }
}

impl From<JamaError> for CliError {
    fn from(e: JamaError) -> Self {
        match &e {
            JamaError::Parse { line, .. } => CliError {
                what: e.to_string(),
                where_: Some(format!("jama.beancount:{line}")),
                fix: Some("fix the syntax at that line and try again".to_string()),
                exit_code: 2,
            },
            JamaError::Imbalance(imb) => CliError {
                what: e.to_string(),
                where_: Some(format!("transaction dated {}", imb.date)),
                fix: Some("add or correct a posting so the transaction sums to zero".to_string()),
                exit_code: 2,
            },
            JamaError::BalanceAssertionFailed { .. } => CliError {
                what: e.to_string(),
                where_: None,
                fix: Some("run `jama check` for details".to_string()),
                exit_code: 2,
            },
            JamaError::UndeclaredAccount { account } => CliError {
                what: e.to_string(),
                where_: None,
                fix: Some(format!(
                    "add `open {account}` to the ledger, or drop --strict"
                )),
                exit_code: 2,
            },
            JamaError::Store(_) => CliError {
                what: e.to_string(),
                where_: None,
                fix: None,
                exit_code: 3,
            },
            JamaError::Import(_) => CliError {
                what: e.to_string(),
                where_: None,
                fix: Some("check the CSV file and rules file paths and column names".to_string()),
                exit_code: 1,
            },
            JamaError::Io { .. } => CliError {
                what: e.to_string(),
                where_: None,
                fix: None,
                exit_code: 3,
            },
            JamaError::Invalid(_) => CliError {
                what: e.to_string(),
                where_: None,
                fix: None,
                exit_code: 1,
            },
        }
    }
}

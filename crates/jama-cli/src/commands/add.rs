use std::io::{self, Write};

use rust_decimal::Decimal;
use std::str::FromStr;

use jama_core::model::{Account, Amount, Posting, Transaction};
use jama_core::{Date, Ledger};

use crate::color::Palette;
use crate::{Cli, CliError};

#[allow(clippy::too_many_arguments)]
pub fn run(
    ledger: &mut Ledger,
    cli: &Cli,
    palette: &Palette,
    narration: &Option<String>,
    amount: &Option<String>,
    from: &Option<String>,
    to: &Option<String>,
    date: &Option<String>,
    commodity: &str,
    payee: &Option<String>,
) -> Result<(), CliError> {
    let (narration, amount, from, to, date, payee) = if narration.is_none() {
        interactive()?
    } else {
        let amount = amount.clone().ok_or_else(|| {
            CliError::user(
                "`jama add` needs an amount",
                "jama add \"Coffee\" 12.50 --from assets:checking --to expenses:cafe",
            )
        })?;
        let from = from
            .clone()
            .ok_or_else(|| CliError::user("`jama add` needs --from", "add --from <account>"))?;
        let to = to
            .clone()
            .ok_or_else(|| CliError::user("`jama add` needs --to", "add --to <account>"))?;
        (
            narration.clone().unwrap(),
            amount,
            from,
            to,
            date.clone(),
            payee.clone(),
        )
    };

    let date = match date {
        Some(s) => Date::parse(&s).map_err(|e| {
            CliError::user(
                format!("bad --date: {e}"),
                "use YYYY-MM-DD, today, or yesterday",
            )
        })?,
        None => Date::today(),
    };
    let amount = Decimal::from_str(&amount).map_err(|e| {
        CliError::user(
            format!("bad amount {amount:?}: {e}"),
            "pass a plain decimal number, e.g. 12.50",
        )
    })?;
    let from_account = Account::parse(&from).map_err(|e| {
        CliError::user(
            e,
            "account must start with assets:/liabilities:/equity:/income:/expenses:",
        )
    })?;
    let to_account = Account::parse(&to).map_err(|e| {
        CliError::user(
            e,
            "account must start with assets:/liabilities:/equity:/income:/expenses:",
        )
    })?;

    let mut txn = Transaction::new(date, narration);
    txn.payee = payee;
    txn.postings
        .push(Posting::new(from_account, Amount::new(-amount, commodity)));
    txn.postings
        .push(Posting::new(to_account, Amount::new(amount, commodity)));

    let id = ledger.add_transaction(txn)?;

    if !cli.quiet {
        if cli.json {
            crate::commands::print_json(&serde_json::json!({ "id": id, "date": date.to_string() }));
        } else {
            let _ = palette;
            println!("added transaction #{id} on {date}");
        }
    }
    Ok(())
}

#[allow(clippy::type_complexity)]
fn interactive() -> Result<
    (
        String,
        String,
        String,
        String,
        Option<String>,
        Option<String>,
    ),
    CliError,
> {
    let narration = prompt("Payee/narration: ")?;
    let amount = prompt("Amount: ")?;
    let from = prompt("From account: ")?;
    let to = prompt("To account: ")?;
    let date = prompt("Date [today]: ")?;
    let date = if date.trim().is_empty() {
        None
    } else {
        Some(date)
    };
    Ok((narration, amount, from, to, date, None))
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
    Ok(line.trim().to_string())
}

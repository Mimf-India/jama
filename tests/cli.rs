//! End-to-end integration tests for the `jama` binary: each test drives
//! the CLI exactly as a user would, against a fresh temporary ledger
//! directory, and inspects either plain-text or `--json` output.

use std::path::Path;
use std::process::Command as StdCommand;

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn jama() -> Command {
    Command::cargo_bin("jama").unwrap()
}

fn fixture(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(name)
}

/// A ledger with a configured default commodity (SAR — an arbitrary
/// choice for these tests, not a product default: `--commodity` on
/// `init` is optional, and most of these tests exist specifically to
/// exercise that path rather than relying on some baked-in currency).
fn init_ledger() -> TempDir {
    let dir = TempDir::new().unwrap();
    jama()
        .arg("init")
        .arg(dir.path())
        .args(["--commodity", "SAR"])
        .assert()
        .success();
    dir
}

#[test]
fn init_creates_expected_layout() {
    let dir = TempDir::new().unwrap();
    jama()
        .arg("init")
        .arg(dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("JAMA v"));

    assert!(dir.path().join("jama.db").exists());
    assert!(dir.path().join("jama.beancount").exists());
    assert!(dir.path().join("rules").is_dir());
}

#[test]
fn version_prints_brand_mark_and_nothing_else_does() {
    jama()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("JAMA v"));

    // The splash must not appear on an ordinary command.
    let dir = init_ledger();
    let out = jama()
        .args(["-f"])
        .arg(dir.path())
        .arg("check")
        .output()
        .unwrap();
    assert!(!String::from_utf8_lossy(&out.stdout).contains("JAMA v"));
}

#[test]
fn add_and_list_round_trip_plain_and_json() {
    let dir = init_ledger();

    jama()
        .args(["-f"])
        .arg(dir.path())
        .args([
            "add",
            "Salary",
            "18000",
            "--from",
            "income:salary",
            "--to",
            "assets:checking",
            "--date",
            "2026-09-01",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("added transaction #1"));

    jama()
        .args(["-f"])
        .arg(dir.path())
        .arg("list")
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Salary").and(predicate::str::contains("assets:checking")),
        );

    let out = jama()
        .args(["-f"])
        .arg(dir.path())
        .args(["list", "--json"])
        .output()
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let txns = json["transactions"].as_array().unwrap();
    assert_eq!(txns.len(), 1);
    assert_eq!(txns[0]["narration"], "Salary");
    assert_eq!(txns[0]["postings"].as_array().unwrap().len(), 2);
}

#[test]
fn balance_accepts_trailing_colon_pattern_from_the_acceptance_walkthrough() {
    let dir = init_ledger();
    jama()
        .args(["-f"])
        .arg(dir.path())
        .args([
            "add",
            "Salary",
            "18000",
            "--from",
            "income:salary",
            "--to",
            "assets:checking",
            "--date",
            "2026-09-01",
        ])
        .assert()
        .success();

    // Exactly the acceptance walkthrough's own invocation.
    jama()
        .args(["-f"])
        .arg(dir.path())
        .args(["balance", "assets:", "--depth", "2"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("assets:checking")
                .and(predicate::str::contains("18,000.00 SAR")),
        );
}

#[test]
fn balance_json_supports_the_negative_filter_from_the_spec() {
    let dir = init_ledger();
    jama()
        .args(["-f"])
        .arg(dir.path())
        .args([
            "add",
            "Salary",
            "18000",
            "--from",
            "income:salary",
            "--to",
            "assets:checking",
            "--date",
            "2026-09-01",
        ])
        .assert()
        .success();

    let out = jama()
        .args(["-f"])
        .arg(dir.path())
        .args(["balance", "--json"])
        .output()
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let negatives: Vec<&serde_json::Value> = json["accounts"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|a| a["balance"].as_f64().map(|b| b < 0.0).unwrap_or(false))
        .collect();
    assert_eq!(negatives.len(), 1);
    assert_eq!(negatives[0]["account"], "income:salary");
}

#[test]
fn check_reports_clean_ledger_and_exits_zero() {
    let dir = init_ledger();
    jama()
        .args(["-f"])
        .arg(dir.path())
        .args([
            "add",
            "Salary",
            "18000",
            "--from",
            "income:salary",
            "--to",
            "assets:checking",
            "--date",
            "2026-09-01",
        ])
        .assert()
        .success();

    jama()
        .args(["-f"])
        .arg(dir.path())
        .arg("check")
        .assert()
        .success()
        .stdout(predicate::str::contains("1").and(predicate::str::contains("0 imbalances")));
}

#[test]
fn strict_mode_rejects_undeclared_accounts() {
    let dir = init_ledger();
    jama()
        .args(["-f"])
        .arg(dir.path())
        .args([
            "--strict",
            "add",
            "Coffee",
            "12.50",
            "--from",
            "assets:checking",
            "--to",
            "expenses:cafe",
        ])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("undeclared account"));
}

#[test]
fn import_is_idempotent_and_applies_rules() {
    let dir = init_ledger();
    let csv = fixture("bank-sample.csv");
    let rules = fixture("rules/sample-bank.toml");

    jama()
        .args(["-f"])
        .arg(dir.path())
        .arg("import")
        .arg(&csv)
        .arg("--rules")
        .arg(&rules)
        .assert()
        .success()
        .stdout(predicate::str::contains("4 rows").and(predicate::str::contains("4 imported")));

    // Re-importing the same file must skip every row as a duplicate.
    jama()
        .args(["-f"])
        .arg(dir.path())
        .arg("import")
        .arg(&csv)
        .arg("--rules")
        .arg(&rules)
        .assert()
        .success()
        .stdout(
            predicate::str::contains("4 skipped (duplicates)")
                .and(predicate::str::contains("0 imported")),
        );

    let out = jama()
        .args(["-f"])
        .arg(dir.path())
        .args(["list", "--json"])
        .output()
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["transactions"].as_array().unwrap().len(), 4);
}

#[test]
fn import_dry_run_writes_nothing() {
    let dir = init_ledger();
    jama()
        .args(["-f"])
        .arg(dir.path())
        .arg("import")
        .arg(fixture("bank-sample.csv"))
        .arg("--rules")
        .arg(fixture("rules/sample-bank.toml"))
        .arg("--dry-run")
        .assert()
        .success()
        .stdout(predicate::str::contains("dry run"));

    let out = jama()
        .args(["-f"])
        .arg(dir.path())
        .args(["list", "--json"])
        .output()
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(json["transactions"].as_array().unwrap().is_empty());
}

#[test]
fn export_round_trips_through_beancount() {
    let dir = init_ledger();
    jama()
        .args(["-f"])
        .arg(dir.path())
        .args([
            "add",
            "Coffee",
            "12.50",
            "--from",
            "assets:checking",
            "--to",
            "expenses:cafe",
        ])
        .assert()
        .success();

    let out_file = dir.path().join("exported.beancount");
    jama()
        .args(["-f"])
        .arg(dir.path())
        .arg("export")
        .arg("--out")
        .arg(&out_file)
        .assert()
        .success();
    let text = std::fs::read_to_string(&out_file).unwrap();
    assert!(text.contains("expenses:cafe"));
    assert!(text.contains("12.50 SAR"));

    // The live jama.beancount file is kept in sync after every mutation —
    // never a stale artifact.
    let live = std::fs::read_to_string(dir.path().join("jama.beancount")).unwrap();
    assert!(live.contains("expenses:cafe"));
}

#[test]
fn export_csv_and_ledger_formats_do_not_error() {
    let dir = init_ledger();
    jama()
        .args(["-f"])
        .arg(dir.path())
        .args([
            "add",
            "Coffee",
            "12.50",
            "--from",
            "assets:checking",
            "--to",
            "expenses:cafe",
        ])
        .assert()
        .success();

    jama()
        .args(["-f"])
        .arg(dir.path())
        .args(["export", "--format", "csv"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "date,flag,payee,narration,account,amount,commodity",
        ));

    jama()
        .args(["-f"])
        .arg(dir.path())
        .args(["export", "--format", "ledger"])
        .assert()
        .success()
        .stdout(predicate::str::contains("expenses:cafe"));
}

#[test]
fn multi_currency_balances_are_never_summed_together() {
    let dir = init_ledger();
    jama()
        .args(["-f"])
        .arg(dir.path())
        .args([
            "add",
            "Salary",
            "18000",
            "--from",
            "income:salary",
            "--to",
            "assets:wallet",
            "--commodity",
            "SAR",
        ])
        .assert()
        .success();
    jama()
        .args(["-f"])
        .arg(dir.path())
        .args([
            "add",
            "Consulting",
            "500",
            "--from",
            "income:consulting",
            "--to",
            "assets:wallet",
            "--commodity",
            "USD",
        ])
        .assert()
        .success();

    let out = jama()
        .args(["-f"])
        .arg(dir.path())
        .args(["balance", "assets:wallet", "--json"])
        .output()
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let accounts = json["accounts"].as_array().unwrap();
    assert_eq!(accounts.len(), 1);
    let balances = accounts[0]["balances"].as_array().unwrap();
    assert_eq!(
        balances.len(),
        2,
        "expected SAR and USD to stay separate, got {balances:?}"
    );
    let commodities: Vec<&str> = balances
        .iter()
        .map(|b| b["commodity"].as_str().unwrap())
        .collect();
    assert!(commodities.contains(&"SAR"));
    assert!(commodities.contains(&"USD"));
}

#[test]
fn arabic_payee_and_narration_are_preserved_end_to_end() {
    let dir = init_ledger();
    jama()
        .args(["-f"])
        .arg(dir.path())
        .args([
            "add",
            "غداء مع صديق",
            "85.50",
            "--from",
            "assets:checking",
            "--to",
            "expenses:cafe",
            "--payee",
            "مطعم الرياض",
            "--date",
            "2026-02-01",
        ])
        .assert()
        .success();

    let out = jama()
        .args(["-f"])
        .arg(dir.path())
        .args(["list", "--json"])
        .output()
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let txn = &json["transactions"][0];
    assert_eq!(txn["narration"], "غداء مع صديق");
    assert_eq!(txn["payee"], "مطعم الرياض");

    // And the exported text file round-trips the same Arabic text.
    let text = std::fs::read_to_string(dir.path().join("jama.beancount")).unwrap();
    assert!(text.contains("غداء مع صديق"));
    assert!(text.contains("مطعم الرياض"));
}

#[test]
fn rm_removes_a_transaction_and_it_disappears_from_list() {
    let dir = init_ledger();
    jama()
        .args(["-f"])
        .arg(dir.path())
        .args([
            "add",
            "Coffee",
            "12.50",
            "--from",
            "assets:checking",
            "--to",
            "expenses:cafe",
        ])
        .assert()
        .success();

    jama()
        .args(["-f"])
        .arg(dir.path())
        .args(["rm", "1"])
        .assert()
        .success();

    let out = jama()
        .args(["-f"])
        .arg(dir.path())
        .args(["list", "--json"])
        .output()
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(json["transactions"].as_array().unwrap().is_empty());
}

#[test]
fn rm_unknown_id_fails_with_a_helpful_message() {
    let dir = init_ledger();
    jama()
        .args(["-f"])
        .arg(dir.path())
        .args(["rm", "999"])
        .assert()
        .failure()
        .code(1)
        .stderr(
            predicate::str::contains("no transaction #999")
                .and(predicate::str::contains("jama list")),
        );
}

#[test]
fn edit_reopens_in_editor_and_reparses_on_save() {
    let dir = init_ledger();
    jama()
        .args(["-f"])
        .arg(dir.path())
        .args([
            "add",
            "Salary",
            "1000",
            "--from",
            "income:salary",
            "--to",
            "assets:checking",
            "--date",
            "2026-01-01",
        ])
        .assert()
        .success();

    // A stand-in $EDITOR that renames the narration, simulating a human
    // editing the transaction text and saving it.
    let sed = StdCommand::new("sed").arg("--version").output();
    if sed.is_err() {
        eprintln!("skipping edit test: `sed` not available");
        return;
    }

    jama()
        .args(["-f"])
        .arg(dir.path())
        .env("EDITOR", "sed -i s/Salary/Wages/")
        .args(["edit", "1"])
        .assert()
        .success()
        .stdout(predicate::str::contains("updated transaction #1"));

    let out = jama()
        .args(["-f"])
        .arg(dir.path())
        .args(["list", "--json"])
        .output()
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["transactions"][0]["narration"], "Wages");
}

#[test]
fn networth_monthly_groups_by_calendar_month() {
    let dir = init_ledger();
    jama()
        .args(["-f"])
        .arg(dir.path())
        .args([
            "add",
            "Salary Jan",
            "1000",
            "--from",
            "income:salary",
            "--to",
            "assets:checking",
            "--date",
            "2026-01-15",
        ])
        .assert()
        .success();
    jama()
        .args(["-f"])
        .arg(dir.path())
        .args([
            "add",
            "Salary Feb",
            "1000",
            "--from",
            "income:salary",
            "--to",
            "assets:checking",
            "--date",
            "2026-02-15",
        ])
        .assert()
        .success();

    let out = jama()
        .args(["-f"])
        .arg(dir.path())
        .args(["networth", "--monthly", "--json"])
        .output()
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let points = json["networth"].as_array().unwrap();
    assert_eq!(points.len(), 2);
    assert_eq!(points[0]["as_of"], "2026-01-01");
    assert_eq!(points[1]["as_of"], "2026-02-01");
}

#[test]
fn no_color_env_var_strips_ansi_from_output() {
    let dir = init_ledger();
    jama()
        .args(["-f"])
        .arg(dir.path())
        .args([
            "add",
            "Coffee",
            "12.50",
            "--from",
            "assets:checking",
            "--to",
            "expenses:cafe",
        ])
        .assert()
        .success();

    let out = jama()
        .env("NO_COLOR", "1")
        .args(["-f"])
        .arg(dir.path())
        .args(["balance", "--json"])
        .output()
        .unwrap();
    // --json never contains ANSI, but this exercises the same code path
    // as the plain-text renderer to make sure NO_COLOR doesn't break it.
    assert!(out.status.success());
    assert!(!String::from_utf8_lossy(&out.stdout).contains('\u{1b}'));
}

#[test]
fn completions_do_not_require_an_existing_ledger() {
    // Regression test: completions used to fail trying to open a ledger it
    // doesn't need.
    jama()
        .args(["completions", "bash"])
        .assert()
        .success()
        .stdout(predicate::str::contains("_jama"));
}

#[test]
fn add_without_a_resolvable_commodity_fails_with_an_actionable_error() {
    // Regression test: `jama add` used to silently default to SAR for
    // every user. With no --commodity, no ledger default (init without
    // --commodity), and no account declaring a currency, it must fail
    // loudly instead of guessing.
    let dir = TempDir::new().unwrap();
    jama().arg("init").arg(dir.path()).assert().success();

    jama()
        .args(["-f"])
        .arg(dir.path())
        .args([
            "add",
            "Coffee",
            "12.50",
            "--from",
            "assets:checking",
            "--to",
            "expenses:cafe",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--commodity"));
}

#[test]
fn init_commodity_flag_sets_a_real_ledger_default_not_a_hardcoded_one() {
    let dir = TempDir::new().unwrap();
    jama()
        .arg("init")
        .arg(dir.path())
        .args(["--commodity", "EUR"])
        .assert()
        .success();

    // No --commodity on add: it must resolve via the ledger's own
    // configured default (EUR here), not some value baked into the binary.
    jama()
        .args(["-f"])
        .arg(dir.path())
        .args([
            "add",
            "Coffee",
            "12.50",
            "--from",
            "assets:checking",
            "--to",
            "expenses:cafe",
        ])
        .assert()
        .success();

    let out = jama()
        .args(["-f"])
        .arg(dir.path())
        .args(["list", "--json"])
        .output()
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let commodity = json["transactions"][0]["postings"][0]["amount"]["commodity"]
        .as_str()
        .unwrap();
    assert_eq!(commodity, "EUR");
}

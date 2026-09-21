# JAMA

**Plain-text accounting as one self-contained binary.**

JAMA (`jama`, alias `jm`) keeps the ownership model of
[Ledger](https://www.ledger-cli.org/)/[hledger](https://hledger.org/)/[Beancount](https://beancount.github.io/) —
human-readable files, double-entry, local-only, no server, no account —
and closes the gap those tools leave you to fill in yourself: a fast
SQLite-backed store, CSV bank import with a learnable rules file, and a
CLI that looks like it was designed rather than assembled from a
parser-combinator tutorial.

**One-line promise:** download one file, run it, and your ledger answers
in milliseconds.

The name is from the Arabic **جمع** — *to gather, to sum*.

## The ownership pitch

Your money's history is yours. JAMA writes it as plain, human-readable
text (`jama.beancount`, a small Beancount-flavoured dialect — see
[`docs/format.md`](docs/format.md)) that you can read with `cat`, diff
with `git`, grep, or hand-edit, forever, with or without JAMA installed.
The SQLite file next to it (`jama.db`) is just a fast index the CLI
happens to use day-to-day — never the thing you're locked into. There is
no server, no account, no telemetry, and no network call anywhere in this
binary: `jama` works the same on a plane as it does at a desk, and it
always will, because there's nothing on the other end to go away.

## Install

**One-liner** (downloads a prebuilt binary for your platform from the
latest [release](../../releases)):

```sh
curl -fsSL https://raw.githubusercontent.com/Mimf-India/jama/main/install.sh | sh
```

**With Cargo**, if you'd rather build from source:

```sh
cargo install --git https://github.com/Mimf-India/jama jama-cli
```

Either way you get a single static binary, `jama` (2–3 MB, well under the
15 MB budget), with no runtime dependencies.

## 60-second tour

```console
$ jama init ~/money
$ jama add "Salary" 18000 --from income:salary --to assets:checking --date 2026-09-01
$ jama import ~/Downloads/bank-sep.csv --rules rules/alrajhi.toml
   142 rows · 138 imported · 4 skipped (duplicates) · 6 unmatched
$ jama balance assets: --depth 2
Account                Balance
------------------------------
assets:checking  18,000.00 SAR

$ jama networth --monthly
Month        Net worth
----------------------
2026-09  18,000.00 SAR

$ jama check
✓ 2 transactions · 0 imbalances · 2 undeclared accounts (warning)

$ jama export --format beancount --out ledger.beancount
$ jama balance --json | jq '.accounts[] | select(.balance < 0)'
```

That's the whole loop: `init` once, `add` or `import` as money moves,
`balance`/`register`/`networth` to see where you stand, `check` to catch
mistakes, `export` whenever you want a portable copy. Every data command
takes `--json` for scripting — see [`docs/json.md`](docs/json.md) for the
exact shapes.

## Command surface

```
jama init [PATH]
jama add [NARRATION] [AMOUNT] [--from ACCT] [--to ACCT] [--date DATE] [--payee P] [--commodity CODE]
jama edit <ID>
jama rm <ID>
jama list [--since] [--until] [--account] [--payee] [--tag] [--limit]
jama balance [PATTERN] [--depth N] [--since] [--until] [--cleared]
jama register <PATTERN> [--since] [--until]
jama networth [--since] [--until] [--monthly]
jama import <FILE.csv> [--rules FILE] [--dry-run]
jama export [--format beancount|ledger|csv] [--out FILE]
jama check
jama completions <shell>
```

Global flags: `-f/--file <ledger dir>` (else `$JAMA_HOME`, else `./jama`,
else `~/.jama`), `--json`, `--no-color`, `--quiet`, `--strict`,
`-V/--version`.

Running `jama add` with no narration drops into an interactive prompt for
payee, amount, accounts, and date. CSV import (see
[`docs/format.md`](docs/format.md#other-export-formats) for the rules
file's tiny matching DSL) is idempotent — re-importing the same file
skips rows it's already seen — and, run interactively, offers to learn a
new rule for any row it can't categorise.

## v0 scope

This is the v0 milestone: record, import, query, report, export, fully
offline. No sync, no budgeting engine, no TUI, no plugins, no MCP server,
no network call — those are follow-on milestones, and the architecture
(a `jama-core` library crate with no I/O chrome, `jama-cli` as a thin
shell on top of it) is deliberately shaped to grow into them without a
rewrite.

## Building from source / development

```sh
git clone https://github.com/Mimf-India/jama
cd jama
cargo build --release          # target/release/jama
cargo test --workspace         # 24 unit tests (jama-core) + 19 integration tests (jama-cli)
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
cargo bench -p jama-core       # criterion benchmarks against the perf budget below
```

Repo layout:

```
jama/
  crates/
    jama-core/     # model, parser, store, reports — no I/O chrome
    jama-core/examples/gen_ledger.rs   # synthetic ledger generator
  crates/jama-cli/ # clap surface, rendering, colour
  benches/         # criterion benchmarks (wired into jama-core's [[bench]] targets)
  tests/           # integration test suite (wired into jama-cli's [[test]] target)
  fixtures/        # sample ledgers incl. multi-currency + Arabic payees
  docs/
    format.md      # the text format and how it maps to Beancount
    json.md         # stable --json schemas
```

## Performance

Measured on a 10,000-transaction synthetic ledger (generate your own with
`cargo run -p jama-core --release --example gen_ledger -- 10000`), against
the budgets this milestone was built to:

| Operation | Budget | Measured |
|---|---|---|
| parse + validate whole file | ≤ 50 ms | ~37 ms |
| `balance` full tree | ≤ 30 ms | ~11 ms |
| `register` on one account | ≤ 20 ms | ~22 ms (worst case: an account touched by every transaction) |
| `import` 1,000 CSV rows | ≤ 200 ms | ~50 ms |

Release binary size: ~2.2 MB (budget 15 MB). Cold start: ~2 ms (budget
20 ms). Numbers will vary by machine — re-run `cargo bench -p jama-core`
on your own hardware for a figure you can trust; CI publishes its own run
on every build.

`register`'s worst case (a query pattern matching essentially every
posting in the ledger — realistic for, say, a primary checking account)
is currently the one figure still slightly over budget; it went from
~66ms to ~22ms during this milestone by replacing full-transaction
hydration with a query that fetches only the columns register actually
displays, and closing the rest of that gap is the next thing to look at.

## License

MIT.

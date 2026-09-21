# `--json` output

Every JAMA command that produces data accepts a global `--json` flag,
which switches its stdout from the plain-text rendering to a single
pretty-printed JSON object. These shapes are considered a stable interface
for scripting (`jama balance --json | jq ...`) — changes to them are
breaking changes, additions of new optional fields are not.

`--json` output never contains ANSI colour codes and is unaffected by
`--no-color` / `NO_COLOR` (there is nothing to strip). Errors are still
written to stderr as plain text, not JSON, in v0.

## Shared shapes

### Amount

Every amount (a balance, a posting's amount, a running total) is
rendered as:

```json
{ "number": "18000.00", "commodity": "SAR", "float": 18000.0 }
```

- `number` is the exact decimal as a string — always safe to parse back
  losslessly, and what you should use for anything that must be exact.
- `float` is a convenience `f64` for quick filtering (e.g.
  `jq 'select(.float < 0)'`); it can lose precision on very large numbers
  and should not be used for anything that needs to be exact.

### Transaction

```json
{
  "id": 1,
  "date": "2026-09-01",
  "flag": "*",
  "payee": null,
  "narration": "Salary",
  "tags": [],
  "links": [],
  "postings": [
    { "account": "income:salary", "amount": { "number": "-18000", "commodity": "SAR", "float": -18000.0 } },
    { "account": "assets:checking", "amount": { "number": "18000", "commodity": "SAR", "float": 18000.0 } }
  ]
}
```

`flag` is `"*"` (cleared) or `"!"` (pending). `payee` is `null` when the
transaction has no distinct payee (just a narration).

## Per-command shapes

### `jama init --json`

```json
{ "root": "/home/you/money", "db": ".../jama.db", "ledger": ".../jama.beancount", "rules_dir": ".../rules" }
```

### `jama add --json`

```json
{ "id": 1, "date": "2026-09-01" }
```

### `jama edit <id> --json`

```json
{ "id": 1, "updated": true }
```

### `jama rm <id> --json`

```json
{ "id": 1, "removed": true }
```

### `jama list --json`

```json
{ "transactions": [ /* Transaction, see above */ ] }
```

### `jama balance --json`

```json
{
  "accounts": [
    {
      "account": "assets:checking",
      "balances": [ /* Amount */ ],
      "balance": 18000.0
    }
  ]
}
```

`balance` is a convenience: the `float` of the account's *first* listed
commodity, or `null` if the account has no balances at all. It exists so
the acceptance example (`jama balance --json | jq '.accounts[] | select(.balance < 0)'`)
works without ceremony on a single-currency ledger. An account holding
more than one commodity still lists every one of them in `balances` —
`balance` never sums across commodities.

### `jama register <pattern> --json`

```json
{
  "entries": [
    {
      "transaction_id": 1,
      "date": "2026-09-01",
      "flag": "*",
      "payee": null,
      "narration": "Salary",
      "account": "assets:checking",
      "amount": { "number": "18000", "commodity": "SAR", "float": 18000.0 },
      "running_balance": [ /* Amount, one per commodity seen so far */ ]
    }
  ]
}
```

### `jama networth --json`

```json
{
  "networth": [
    { "as_of": "2026-09-01", "balances": [ /* Amount */ ] }
  ]
}
```

Without `--monthly` this is a single-element array (net worth as of the
latest transaction in range). With `--monthly`, one element per calendar
month that had at least one transaction, `as_of` being that month's first
day.

### `jama check --json`

```json
{
  "transactions_checked": 1204,
  "imbalances": [],
  "failed_assertions": [],
  "undeclared_accounts": ["assets:new-wallet"],
  "clean": true
}
```

`clean` is `imbalances.is_empty() && failed_assertions.is_empty()` —
undeclared accounts are always a warning, never a reason `clean` is
`false`. The process still exits `2` whenever `clean` is `false`, exactly
as it does without `--json`.

### `jama import --json`

```json
{
  "total_rows": 142,
  "imported": 138,
  "skipped_duplicates": 4,
  "categorized_interactively": 0,
  "unmatched": 0,
  "dry_run": false
}
```

Interactive categorisation of unmatched rows only happens in plain-text
mode with a TTY attached; under `--json` (or `--quiet`), unmatched rows
are just counted and left for a follow-up import once you've extended the
rules file.

### `jama export --out FILE --json`

```json
{ "written_to": "/home/you/money/ledger.beancount" }
```

(`jama export` without `--out` writes the exported text itself to stdout,
in whichever `--format` was requested — `--json` has no effect there,
since the export text isn't JSON-shaped data to begin with.)

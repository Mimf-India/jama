# The JAMA ledger text format

`jama.beancount` is the file every ledger directory keeps up to date after
every command that changes anything (`add`, `edit`, `rm`, `import`). It is
JAMA's source of truth for interchange: the SQLite `jama.db` file next to it
is a fast working cache the CLI actually reads and writes day-to-day, but
you should always be able to delete `jama.db`, keep only `jama.beancount`,
and reconstruct everything from it (a "reimport" path is on the roadmap
past v0 — for now, treat the text file as the thing worth backing up and
committing to git).

The syntax is a deliberately small subset of
[Beancount](https://beancount.github.io/docs/beancount_language_syntax.html):
if you already know Beancount, everything below will look familiar. It is
**not** a full Beancount implementation — no `include`, no `push`/`pop_tag`,
no plugins, no full booking/inventory algebra — just what a v0 personal
ledger needs.

## Directives

Every directive is a single top-level line starting with a date in
`YYYY-MM-DD` form, followed by a keyword (or a transaction flag) and its
arguments. Blank lines and lines starting with `;` are ignored.

### Transactions

```
2026-09-01 * "Cafe Bateel" "Coffee with a friend" #social
  assets:checking  -12.50 SAR
  expenses:cafe  12.50 SAR
```

- The flag is `*` (cleared) or `!` (pending).
- One or two quoted strings follow the flag: `"narration"` alone, or
  `"payee" "narration"`.
- `#tag` and `^link` tokens may follow the strings, space-separated.
- Each indented line under the header is either a **posting** (an account
  followed by an optional amount) or a **metadata** line (`key: "value"`).
- Every transaction must have at least two postings. At most one posting
  per transaction may omit its amount — JAMA fills it in as whatever makes
  that transaction's single unbalanced commodity sum to zero. If more than
  one commodity is unbalanced when a posting is elided, that's an error:
  JAMA won't guess which currency the missing leg should be in.
- A transaction must sum to exactly zero **within each commodity**.
  Amounts in different commodities are never implicitly converted or
  summed together — see [Multi-currency](#multi-currency) below.

### `open` — declare an account

```
2026-01-01 open assets:checking SAR
2026-01-01 open assets:wallet
```

Declaring an account is optional in v0's forgiving-by-default mode
(`jama check` reports postings to undeclared accounts as a warning, not an
error), but required under `--strict`. The currency list after the account
is informational for now (a hint for tooling, not yet enforced).

### `close` — retire an account

```
2026-06-01 close assets:old-wallet
```

### `balance` — assert a running balance

```
2026-09-30 balance assets:checking  4,231.00 SAR
```

`jama check` recomputes the account's running balance up to (and
including) this date and reports a failure if it doesn't match. Useful for
catching a typo or a missed transaction against your bank's own statement
balance.

### `pad` — a shorthand for "the difference goes here"

```
2026-01-01 pad assets:checking equity:opening-balances
2026-01-02 balance assets:checking  1,000.00 SAR
```

Declares that the account named should absorb whatever difference is
needed to satisfy the *next* `balance` assertion on the same account —
handy for recording a starting balance without knowing its exact history.

### `commodity` — declare a currency's display precision

```
2026-01-01 commodity SAR
  precision: "2"
```

## Accounts

An account is a colon-separated path that must start with one of the five
roots: `assets:`, `liabilities:`, `equity:`, `income:`, `expenses:`. Each
segment may contain letters, digits, `-`, and `_` (Unicode letters are
fine — Arabic account names work the same as ASCII ones).

`jama balance assets:` and `jama balance assets` are equivalent: a
trailing colon on a pattern is stripped before matching, so it always
means "this account or anything nested under it."

## Amounts and precision

Amounts are always exact decimals (`rust_decimal`, fixed-point), never
floats — arithmetic on money never accumulates binary-floating-point
error. Each commodity has a declared display precision (`commodity`
directive; defaults to 2 if undeclared) used when rendering, not when
storing: internally JAMA keeps whatever precision you typed.

## Multi-currency

A single account can hold balances in more than one commodity — JAMA
tracks each commodity's running total separately and never adds, say, USD
and SAR into one number. `jama balance` and `jama balance --json` list
every commodity an account holds as separate entries. If you want to
record a currency conversion, post the SAR leg and the USD leg as two
separate, individually-balanced transactions (each summing to zero in its
own commodity) rather than one two-posting transaction mixing currencies —
JAMA has no implicit FX rate table in v0, so it won't try to guess one for
you.

## Comments

A line starting with `;` is skipped by the parser. JAMA does not currently
preserve comments through a re-export (`jama export` regenerates
`jama.beancount` from the database, which has no comment field) — if you
hand-annotate your ledger file, keep those notes in a separate file, or as
transaction/posting metadata (`key: "value"` lines), which *do* round-trip.

## Round-tripping

`jama export --format beancount` (and the `jama.beancount` file kept
alongside `jama.db`) is produced by the same serializer the parser is the
inverse of: parsing that output and re-serializing it produces
byte-identical text. This is exercised directly in
`crates/jama-core/src/parser.rs`'s test suite, including on the
multi-currency and Arabic-payee ledgers under `fixtures/`.

## Other export formats

`jama export --format ledger` writes a close approximation of
[ledger-cli](https://www.ledger-cli.org/)'s own text syntax (payee and
narration joined onto one line, since ledger-cli doesn't distinguish them
the way Beancount does). `jama export --format csv` flattens every posting
to one CSV row (`date,flag,payee,narration,account,amount,commodity`) —
both are one-way exports for interoperating with other tools, not meant to
be re-imported by JAMA itself.

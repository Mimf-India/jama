//! Shared helpers for building the stable `--json` output shapes
//! documented in `docs/json.md`. Kept in the CLI crate (not `jama-core`)
//! because JSON is a presentation concern, not part of the core model.

use rust_decimal::prelude::ToPrimitive;
use serde_json::{json, Value};

use jama_core::model::{Amount, Posting, Transaction};

/// `{"number": "12.50", "commodity": "SAR", "float": 12.5}` — `number` is
/// the exact decimal string (always safe to parse back losslessly);
/// `float` is a convenience for quick filtering (e.g. `jq '... < 0'`) and
/// may lose precision for very large numbers.
pub fn amount_json(a: &Amount) -> Value {
    json!({
        "number": a.number.to_string(),
        "commodity": a.commodity,
        "float": a.number.to_f64(),
    })
}

pub fn posting_json(p: &Posting) -> Value {
    json!({
        "account": p.account.as_str(),
        "amount": p.amount.as_ref().map(amount_json),
    })
}

pub fn transaction_json(t: &Transaction) -> Value {
    json!({
        "id": t.id,
        "date": t.date.to_string(),
        "flag": t.flag.as_char().to_string(),
        "payee": t.payee,
        "narration": t.narration,
        "tags": t.tags,
        "links": t.links,
        "postings": t.postings.iter().map(posting_json).collect::<Vec<_>>(),
    })
}

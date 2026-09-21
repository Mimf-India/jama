//! `jama-core`: the JAMA data model, text-format parser/serializer, SQLite
//! store, CSV importer, exporter, and reports. No CLI, no colour, no I/O
//! chrome — `jama-cli` is the thin shell built on top of this.

pub mod csv_import;
pub mod date;
pub mod error;
pub mod export;
pub mod ledger;
pub mod model;
pub mod parser;
pub mod reports;
pub mod store;
pub mod testkit;

pub use date::Date;
pub use error::{JamaError, Result};
pub use ledger::Ledger;

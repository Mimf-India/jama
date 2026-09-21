//! The `Ledger`: ties together the on-disk layout (`jama.db` +
//! `jama.beancount` + `rules/`), the SQLite [`Store`], and the text
//! [`parser`]/[`export`] so callers never have to juggle the pieces
//! themselves.

use std::fs;
use std::path::{Path, PathBuf};

use crate::date::Date;
use crate::error::{JamaError, Result};
use crate::export::ExportFormat;
use crate::model::{Account, CommodityInfo, Directive, Transaction};
use crate::store::{ListFilter, Store};
use crate::{export, parser};

/// The on-disk layout of a ledger directory.
#[derive(Debug, Clone)]
pub struct Paths {
    pub root: PathBuf,
    pub db: PathBuf,
    pub text: PathBuf,
    pub rules_dir: PathBuf,
}

impl Paths {
    pub fn for_root(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            db: root.join("jama.db"),
            text: root.join("jama.beancount"),
            rules_dir: root.join("rules"),
        }
    }

    /// Resolve the ledger directory the same way every command does:
    /// `--file/-f`, else `$JAMA_HOME`, else `./jama`, else `~/.jama`.
    pub fn resolve(explicit: Option<&Path>) -> PathBuf {
        if let Some(p) = explicit {
            return p.to_path_buf();
        }
        if let Ok(home_env) = std::env::var("JAMA_HOME") {
            if !home_env.is_empty() {
                return PathBuf::from(home_env);
            }
        }
        let local = PathBuf::from("./jama");
        if local.exists() {
            return local;
        }
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(".jama");
        }
        local
    }
}

pub struct Ledger {
    pub paths: Paths,
    pub store: Store,
    pub strict: bool,
}

impl Ledger {
    /// Create a brand-new ledger directory: `jama.db`, an empty
    /// `jama.beancount`, and a `rules/` folder for CSV import rules.
    pub fn init(root: &Path) -> Result<Self> {
        fs::create_dir_all(root).map_err(|e| JamaError::Io {
            context: format!("creating {}", root.display()),
            source: e,
        })?;
        let paths = Paths::for_root(root);
        fs::create_dir_all(&paths.rules_dir).map_err(|e| JamaError::Io {
            context: "creating rules/".to_string(),
            source: e,
        })?;
        let store = Store::open(&paths.db)?;
        if !paths.text.exists() {
            fs::write(&paths.text, "").map_err(|e| JamaError::Io {
                context: "writing jama.beancount".to_string(),
                source: e,
            })?;
        }
        Ok(Self {
            paths,
            store,
            strict: false,
        })
    }

    /// Open an existing ledger directory.
    pub fn open(root: &Path) -> Result<Self> {
        let paths = Paths::for_root(root);
        if !paths.db.exists() {
            return Err(JamaError::Invalid(format!(
                "no ledger found at {} — run `jama init {}` first",
                root.display(),
                root.display()
            )));
        }
        let store = Store::open(&paths.db)?;
        Ok(Self {
            paths,
            store,
            strict: false,
        })
    }

    pub fn with_strict(mut self, strict: bool) -> Self {
        self.strict = strict;
        self
    }

    // -- mutation ------------------------------------------------------

    /// Validate, resolve any elided posting, persist, and refresh the text
    /// export. Returns the new transaction id.
    pub fn add_transaction(&mut self, mut txn: Transaction) -> Result<i64> {
        let resolved = txn.resolve_postings().map_err(JamaError::Imbalance)?;
        txn.postings = resolved;

        if self.strict {
            for p in &txn.postings {
                if !self.store.is_account_declared(p.account.as_str())? {
                    return Err(JamaError::UndeclaredAccount {
                        account: p.account.to_string(),
                    });
                }
            }
        }

        let id = self.store.insert_transaction(&txn)?;
        self.sync_text_file()?;
        Ok(id)
    }

    /// Same as [`Self::add_transaction`] but skips the text-file sync —
    /// used by bulk importers that sync once at the end.
    pub fn add_transaction_no_sync(&mut self, mut txn: Transaction) -> Result<i64> {
        let resolved = txn.resolve_postings().map_err(JamaError::Imbalance)?;
        txn.postings = resolved;
        self.store.insert_transaction(&txn)
    }

    pub fn update_transaction(&mut self, id: i64, mut txn: Transaction) -> Result<()> {
        let resolved = txn.resolve_postings().map_err(JamaError::Imbalance)?;
        txn.postings = resolved;
        self.store.update_transaction(id, &txn)?;
        self.sync_text_file()?;
        Ok(())
    }

    pub fn remove_transaction(&mut self, id: i64) -> Result<bool> {
        let removed = self.store.delete_transaction(id)?;
        if removed {
            self.sync_text_file()?;
        }
        Ok(removed)
    }

    pub fn get_transaction(&self, id: i64) -> Result<Option<Transaction>> {
        self.store.get_transaction(id)
    }

    pub fn declare_account(
        &self,
        date: Date,
        account: &Account,
        currencies: &[String],
    ) -> Result<()> {
        self.store.declare_account(date, account, currencies)
    }

    pub fn declare_commodity(&self, date: Date, info: &CommodityInfo) -> Result<()> {
        let _ = date;
        self.store.declare_commodity(info)
    }

    pub fn insert_balance_assertion(
        &self,
        date: Date,
        account: &Account,
        amount: &crate::model::Amount,
    ) -> Result<()> {
        self.store.insert_balance_assertion(date, account, amount)
    }

    pub fn insert_pad(&self, date: Date, account: &Account, source: &Account) -> Result<()> {
        self.store.insert_pad(date, account, source)
    }

    // -- queries ---------------------------------------------------------

    pub fn list_transactions(&self, filter: &ListFilter) -> Result<Vec<Transaction>> {
        self.store.list_transactions(filter)
    }

    pub fn all_transactions(&self) -> Result<Vec<Transaction>> {
        self.store.all_transactions()
    }

    // -- text file sync ----------------------------------------------------

    /// Rewrite `jama.beancount` from the current store contents. Called
    /// after every mutation so the text file is always ready to be moved
    /// elsewhere — it is never a stale artifact you have to remember to
    /// regenerate.
    pub fn sync_text_file(&self) -> Result<()> {
        let text = export::render(&self.store, ExportFormat::Beancount)?;
        fs::write(&self.paths.text, text).map_err(|e| JamaError::Io {
            context: "writing jama.beancount".to_string(),
            source: e,
        })
    }

    /// Re-import `jama.beancount` from disk (used by `jama edit`, which
    /// hands the user the text file to modify and re-parses it on save).
    pub fn reload_from_text_file(&mut self) -> Result<Vec<Directive>> {
        let text = fs::read_to_string(&self.paths.text).map_err(|e| JamaError::Io {
            context: "reading jama.beancount".to_string(),
            source: e,
        })?;
        parser::parse(&text)
    }
}

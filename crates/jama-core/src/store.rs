//! The SQLite working store. This is the fast, queryable index JAMA
//! actually reads and writes day-to-day; the Beancount-compatible text file
//! (see [`crate::export`] / [`crate::parser`]) is the source of truth for
//! interchange, never this database.

use std::collections::BTreeMap;
use std::path::Path;
use std::str::FromStr;

use rusqlite::{params, Connection, OptionalExtension};
use rust_decimal::Decimal;

use crate::date::Date;
use crate::error::{JamaError, Result};
use crate::model::{Account, Amount, CommodityInfo, CostOrPrice, Flag, Posting, Transaction};

pub struct Store {
    conn: Connection,
}

/// `(account, opened_on, closed_on, currencies)`.
pub type AccountDeclaration = (Account, Date, Option<Date>, Vec<String>);

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS accounts (
    name TEXT PRIMARY KEY,
    opened_on TEXT NOT NULL,
    closed_on TEXT,
    currencies TEXT NOT NULL DEFAULT ''
);

CREATE TABLE IF NOT EXISTS commodities (
    code TEXT PRIMARY KEY,
    precision INTEGER NOT NULL DEFAULT 2
);

CREATE TABLE IF NOT EXISTS transactions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    date TEXT NOT NULL,
    flag TEXT NOT NULL,
    payee TEXT,
    narration TEXT NOT NULL,
    tags TEXT NOT NULL DEFAULT '',
    links TEXT NOT NULL DEFAULT '',
    meta TEXT NOT NULL DEFAULT '{}'
);
CREATE INDEX IF NOT EXISTS idx_transactions_date ON transactions(date);

CREATE TABLE IF NOT EXISTS postings (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    transaction_id INTEGER NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
    ord INTEGER NOT NULL,
    account TEXT NOT NULL,
    number TEXT NOT NULL,
    commodity TEXT NOT NULL,
    cost_number TEXT,
    cost_commodity TEXT,
    is_cost INTEGER
);
CREATE INDEX IF NOT EXISTS idx_postings_txn ON postings(transaction_id);
CREATE INDEX IF NOT EXISTS idx_postings_account ON postings(account);

CREATE TABLE IF NOT EXISTS balance_assertions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    date TEXT NOT NULL,
    account TEXT NOT NULL,
    number TEXT NOT NULL,
    commodity TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS pads (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    date TEXT NOT NULL,
    account TEXT NOT NULL,
    source TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS import_dedupe (
    hash TEXT PRIMARY KEY,
    transaction_id INTEGER,
    imported_at TEXT NOT NULL
);
"#;

/// Filters accepted by [`Store::list_transactions`]; all are optional and
/// AND together.
#[derive(Debug, Default, Clone)]
pub struct ListFilter {
    pub since: Option<Date>,
    pub until: Option<Date>,
    pub account: Option<String>,
    pub payee: Option<String>,
    pub tag: Option<String>,
    pub limit: Option<u32>,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)
            .map_err(|e| JamaError::Store(format!("opening {}: {e}", path.display())))?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self { conn })
    }

    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self { conn })
    }

    // -- accounts --------------------------------------------------------

    pub fn declare_account(
        &self,
        date: Date,
        account: &Account,
        currencies: &[String],
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO accounts (name, opened_on, currencies) VALUES (?1, ?2, ?3)
             ON CONFLICT(name) DO UPDATE SET opened_on = excluded.opened_on, currencies = excluded.currencies",
            params![account.as_str(), date.to_string(), currencies.join(",")],
        )?;
        Ok(())
    }

    pub fn close_account(&self, date: Date, account: &Account) -> Result<()> {
        self.conn.execute(
            "UPDATE accounts SET closed_on = ?1 WHERE name = ?2",
            params![date.to_string(), account.as_str()],
        )?;
        Ok(())
    }

    pub fn is_account_declared(&self, account: &str) -> Result<bool> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM accounts WHERE name = ?1",
            params![account],
            |r| r.get(0),
        )?;
        Ok(count > 0)
    }

    pub fn list_declared_accounts(&self) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT name FROM accounts ORDER BY name")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(JamaError::from)
    }

    /// Full account declarations: (account, opened_on, closed_on, currencies).
    pub fn list_account_declarations(&self) -> Result<Vec<AccountDeclaration>> {
        let mut stmt = self
            .conn
            .prepare("SELECT name, opened_on, closed_on, currencies FROM accounts ORDER BY name")?;
        let rows = stmt.query_map([], |r| {
            let name: String = r.get(0)?;
            let opened: String = r.get(1)?;
            let closed: Option<String> = r.get(2)?;
            let currencies: String = r.get(3)?;
            Ok((name, opened, closed, currencies))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (name, opened, closed, currencies) = row?;
            let account = Account::parse(&name).map_err(JamaError::Store)?;
            let opened = Date::parse_iso(&opened).map_err(JamaError::Store)?;
            let closed = match closed {
                Some(c) => Some(Date::parse_iso(&c).map_err(JamaError::Store)?),
                None => None,
            };
            let currencies: Vec<String> = if currencies.is_empty() {
                Vec::new()
            } else {
                currencies.split(',').map(str::to_string).collect()
            };
            out.push((account, opened, closed, currencies));
        }
        Ok(out)
    }

    pub fn list_commodities(&self) -> Result<Vec<CommodityInfo>> {
        let mut stmt = self
            .conn
            .prepare("SELECT code, precision FROM commodities ORDER BY code")?;
        let rows = stmt.query_map([], |r| {
            let code: String = r.get(0)?;
            let precision: u8 = r.get(1)?;
            Ok(CommodityInfo::new(code, precision))
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(JamaError::from)
    }

    pub fn list_pads(&self) -> Result<Vec<(Date, Account, Account)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT date, account, source FROM pads ORDER BY date")?;
        let rows = stmt.query_map([], |r| {
            let date: String = r.get(0)?;
            let account: String = r.get(1)?;
            let source: String = r.get(2)?;
            Ok((date, account, source))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (date, account, source) = row?;
            out.push((
                Date::parse_iso(&date).map_err(JamaError::Store)?,
                Account::parse(&account).map_err(JamaError::Store)?,
                Account::parse(&source).map_err(JamaError::Store)?,
            ));
        }
        Ok(out)
    }

    // -- commodities -------------------------------------------------------

    pub fn declare_commodity(&self, info: &CommodityInfo) -> Result<()> {
        self.conn.execute(
            "INSERT INTO commodities (code, precision) VALUES (?1, ?2)
             ON CONFLICT(code) DO UPDATE SET precision = excluded.precision",
            params![info.code, info.precision],
        )?;
        Ok(())
    }

    pub fn commodity_precision(&self, code: &str) -> Result<u8> {
        let precision: Option<u8> = self
            .conn
            .query_row(
                "SELECT precision FROM commodities WHERE code = ?1",
                params![code],
                |r| r.get(0),
            )
            .optional()?;
        Ok(precision.unwrap_or(2))
    }

    // -- transactions ------------------------------------------------------

    pub fn insert_transaction(&self, t: &Transaction) -> Result<i64> {
        let meta_json = serde_json::to_string(&t.meta).unwrap_or_else(|_| "{}".to_string());
        self.conn.execute(
            "INSERT INTO transactions (date, flag, payee, narration, tags, links, meta)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                t.date.to_string(),
                t.flag.as_char().to_string(),
                t.payee,
                t.narration,
                t.tags.join(","),
                t.links.join(","),
                meta_json,
            ],
        )?;
        let id = self.conn.last_insert_rowid();
        self.insert_postings(id, &t.postings)?;
        Ok(id)
    }

    fn insert_postings(&self, transaction_id: i64, postings: &[Posting]) -> Result<()> {
        for (ord, p) in postings.iter().enumerate() {
            let amount = p.amount.as_ref().ok_or_else(|| {
                JamaError::Store("cannot persist a posting with an unresolved amount".to_string())
            })?;
            let (cost_number, cost_commodity, is_cost) = match &p.cost_or_price {
                Some(cp) => (
                    Some(cp.amount.number.to_string()),
                    Some(cp.amount.commodity.clone()),
                    Some(cp.is_cost as i64),
                ),
                None => (None, None, None),
            };
            self.conn.execute(
                "INSERT INTO postings (transaction_id, ord, account, number, commodity, cost_number, cost_commodity, is_cost)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    transaction_id,
                    ord as i64,
                    p.account.as_str(),
                    amount.number.to_string(),
                    amount.commodity,
                    cost_number,
                    cost_commodity,
                    is_cost,
                ],
            )?;
        }
        Ok(())
    }

    pub fn update_transaction(&self, id: i64, t: &Transaction) -> Result<()> {
        let meta_json = serde_json::to_string(&t.meta).unwrap_or_else(|_| "{}".to_string());
        self.conn.execute(
            "UPDATE transactions SET date=?1, flag=?2, payee=?3, narration=?4, tags=?5, links=?6, meta=?7 WHERE id=?8",
            params![
                t.date.to_string(),
                t.flag.as_char().to_string(),
                t.payee,
                t.narration,
                t.tags.join(","),
                t.links.join(","),
                meta_json,
                id,
            ],
        )?;
        self.conn.execute(
            "DELETE FROM postings WHERE transaction_id = ?1",
            params![id],
        )?;
        self.insert_postings(id, &t.postings)?;
        Ok(())
    }

    pub fn delete_transaction(&self, id: i64) -> Result<bool> {
        let n = self
            .conn
            .execute("DELETE FROM transactions WHERE id = ?1", params![id])?;
        Ok(n > 0)
    }

    pub fn get_transaction(&self, id: i64) -> Result<Option<Transaction>> {
        let mut txn = self
            .conn
            .query_row(
                "SELECT id, date, flag, payee, narration, tags, links, meta FROM transactions WHERE id = ?1",
                params![id],
                row_to_transaction,
            )
            .optional()?;
        if let Some(t) = txn.as_mut() {
            t.postings = self.load_postings(id)?;
        }
        Ok(txn)
    }

    fn load_postings(&self, transaction_id: i64) -> Result<Vec<Posting>> {
        let mut stmt = self.conn.prepare(
            "SELECT account, number, commodity, cost_number, cost_commodity, is_cost
             FROM postings WHERE transaction_id = ?1 ORDER BY ord",
        )?;
        let rows = stmt.query_map(params![transaction_id], row_to_posting)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(JamaError::from)
    }

    pub fn list_transactions(&self, filter: &ListFilter) -> Result<Vec<Transaction>> {
        let mut sql = String::from(
            "SELECT id, date, flag, payee, narration, tags, links, meta FROM transactions WHERE 1=1",
        );
        let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        if let Some(since) = &filter.since {
            sql.push_str(" AND date >= ?");
            args.push(Box::new(since.to_string()));
        }
        if let Some(until) = &filter.until {
            sql.push_str(" AND date <= ?");
            args.push(Box::new(until.to_string()));
        }
        if let Some(payee) = &filter.payee {
            sql.push_str(" AND payee LIKE ?");
            args.push(Box::new(format!("%{payee}%")));
        }
        if let Some(tag) = &filter.tag {
            sql.push_str(" AND (',' || tags || ',') LIKE ?");
            args.push(Box::new(format!("%,{tag},%")));
        }
        if let Some(account) = &filter.account {
            sql.push_str(
                " AND id IN (SELECT transaction_id FROM postings WHERE account = ? OR account LIKE ?)",
            );
            args.push(Box::new(account.clone()));
            args.push(Box::new(format!("{account}:%")));
        }
        sql.push_str(" ORDER BY date, id");
        if let Some(limit) = filter.limit {
            sql.push_str(&format!(" LIMIT {limit}"));
        }

        let mut stmt = self.conn.prepare(&sql)?;
        let param_refs: Vec<&dyn rusqlite::ToSql> = args.iter().map(|b| b.as_ref()).collect();
        let mut txns: Vec<Transaction> = stmt
            .query_map(param_refs.as_slice(), row_to_transaction)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        for t in &mut txns {
            t.postings = self.load_postings(t.id.unwrap())?;
        }
        Ok(txns)
    }

    pub fn all_transactions(&self) -> Result<Vec<Transaction>> {
        self.list_transactions(&ListFilter::default())
    }

    // -- balance assertions / pads ------------------------------------------

    pub fn insert_balance_assertion(
        &self,
        date: Date,
        account: &Account,
        amount: &Amount,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO balance_assertions (date, account, number, commodity) VALUES (?1, ?2, ?3, ?4)",
            params![date.to_string(), account.as_str(), amount.number.to_string(), amount.commodity],
        )?;
        Ok(())
    }

    pub fn list_balance_assertions(&self) -> Result<Vec<(Date, Account, Amount)>> {
        let mut stmt = self.conn.prepare(
            "SELECT date, account, number, commodity FROM balance_assertions ORDER BY date",
        )?;
        let rows = stmt.query_map([], |r| {
            let date: String = r.get(0)?;
            let account: String = r.get(1)?;
            let number: String = r.get(2)?;
            let commodity: String = r.get(3)?;
            Ok((date, account, number, commodity))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (date, account, number, commodity) = row?;
            out.push((
                Date::parse_iso(&date).map_err(JamaError::Store)?,
                Account::parse(&account).map_err(JamaError::Store)?,
                Amount::new(
                    Decimal::from_str(&number).map_err(|e| JamaError::Store(e.to_string()))?,
                    commodity,
                ),
            ));
        }
        Ok(out)
    }

    pub fn insert_pad(&self, date: Date, account: &Account, source: &Account) -> Result<()> {
        self.conn.execute(
            "INSERT INTO pads (date, account, source) VALUES (?1, ?2, ?3)",
            params![date.to_string(), account.as_str(), source.as_str()],
        )?;
        Ok(())
    }

    // -- import dedupe -------------------------------------------------------

    pub fn has_import_hash(&self, hash: &str) -> Result<bool> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM import_dedupe WHERE hash = ?1",
            params![hash],
            |r| r.get(0),
        )?;
        Ok(count > 0)
    }

    pub fn record_import_hash(&self, hash: &str, transaction_id: i64) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO import_dedupe (hash, transaction_id, imported_at) VALUES (?1, ?2, ?3)",
            params![hash, transaction_id, Date::today().to_string()],
        )?;
        Ok(())
    }

    /// Run `f` inside a SQLite transaction, committing on success.
    pub fn with_transaction<T>(&mut self, f: impl FnOnce(&Store) -> Result<T>) -> Result<T> {
        self.conn.execute_batch("BEGIN")?;
        match f(self) {
            Ok(v) => {
                self.conn.execute_batch("COMMIT")?;
                Ok(v)
            }
            Err(e) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(e)
            }
        }
    }
}

fn row_to_transaction(row: &rusqlite::Row) -> rusqlite::Result<Transaction> {
    let id: i64 = row.get(0)?;
    let date: String = row.get(1)?;
    let flag: String = row.get(2)?;
    let payee: Option<String> = row.get(3)?;
    let narration: String = row.get(4)?;
    let tags: String = row.get(5)?;
    let links: String = row.get(6)?;
    let meta: String = row.get(7)?;

    let date = Date::parse_iso(&date).unwrap_or_else(|_| Date::today());
    let flag = Flag::parse(flag.chars().next().unwrap_or('*')).unwrap_or(Flag::Cleared);
    let tags = if tags.is_empty() {
        Vec::new()
    } else {
        tags.split(',').map(str::to_string).collect()
    };
    let links = if links.is_empty() {
        Vec::new()
    } else {
        links.split(',').map(str::to_string).collect()
    };
    let meta: BTreeMap<String, String> = serde_json::from_str(&meta).unwrap_or_default();

    Ok(Transaction {
        id: Some(id),
        date,
        flag,
        payee,
        narration,
        tags,
        links,
        postings: Vec::new(),
        meta,
    })
}

fn row_to_posting(row: &rusqlite::Row) -> rusqlite::Result<Posting> {
    let account: String = row.get(0)?;
    let number: String = row.get(1)?;
    let commodity: String = row.get(2)?;
    let cost_number: Option<String> = row.get(3)?;
    let cost_commodity: Option<String> = row.get(4)?;
    let is_cost: Option<i64> = row.get(5)?;

    let account =
        Account::parse(&account).unwrap_or_else(|_| Account::parse("expenses:unknown").unwrap());
    let amount = Amount::new(Decimal::from_str(&number).unwrap_or_default(), commodity);
    let cost_or_price = match (cost_number, cost_commodity, is_cost) {
        (Some(n), Some(c), Some(flag)) => Some(CostOrPrice {
            amount: Amount::new(Decimal::from_str(&n).unwrap_or_default(), c),
            is_cost: flag != 0,
        }),
        _ => None,
    };

    Ok(Posting {
        account,
        amount: Some(amount),
        cost_or_price,
        meta: BTreeMap::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Posting as ModelPosting;

    fn sample_txn() -> Transaction {
        let date = Date::from_ymd(2026, 9, 1).unwrap();
        let mut t = Transaction::new(date, "Salary");
        t.postings.push(ModelPosting::new(
            Account::parse("income:salary").unwrap(),
            Amount::new(Decimal::from_str("-18000").unwrap(), "SAR"),
        ));
        t.postings.push(ModelPosting::new(
            Account::parse("assets:checking").unwrap(),
            Amount::new(Decimal::from_str("18000").unwrap(), "SAR"),
        ));
        t
    }

    #[test]
    fn insert_and_fetch_round_trips() {
        let store = Store::open_in_memory().unwrap();
        let id = store.insert_transaction(&sample_txn()).unwrap();
        let fetched = store.get_transaction(id).unwrap().unwrap();
        assert_eq!(fetched.narration, "Salary");
        assert_eq!(fetched.postings.len(), 2);
    }

    #[test]
    fn dedupe_hash_is_tracked() {
        let store = Store::open_in_memory().unwrap();
        assert!(!store.has_import_hash("abc").unwrap());
        let id = store.insert_transaction(&sample_txn()).unwrap();
        store.record_import_hash("abc", id).unwrap();
        assert!(store.has_import_hash("abc").unwrap());
    }

    #[test]
    fn list_filters_by_account() {
        let store = Store::open_in_memory().unwrap();
        store.insert_transaction(&sample_txn()).unwrap();
        let filter = ListFilter {
            account: Some("assets:checking".to_string()),
            ..Default::default()
        };
        let results = store.list_transactions(&filter).unwrap();
        assert_eq!(results.len(), 1);

        let filter = ListFilter {
            account: Some("liabilities".to_string()),
            ..Default::default()
        };
        assert!(store.list_transactions(&filter).unwrap().is_empty());
    }
}

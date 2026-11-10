//! Response cache in SQLite (the user data folder), with lifetimes within
//! each provider's terms. Kinds: `search` and `part` hold prices and stock,
//! `params` holds parameters only.
//!
//! | Provider | Lifetime | Why |
//! | --- | --- | --- |
//! | JLCPCB (jlcsearch) | 24 h | its README asks clients to revalidate within 24 h (nightly stock sync) |
//! | DigiKey | 24 h | API User Agreement 5.1(e): no own database of DigiKey data; no lifetime stated |
//! | Mouser | 24 h | terms pages could not be read; the spec default for price and stock |
//! | TME | 24 h (search, prices), 30 days (parameters) | terms 2026-07-01 permit use in native desktop apps; no lifetime stated |
//! | Farnell | none | API Terms of Use 4A: do not cache, record or store Farnell Content |

use crate::ProviderId;
use std::sync::Mutex;

/// Seconds a cached entry of `kind` stays valid, or None (never cache).
pub fn lifetime(p: ProviderId, kind: &str) -> Option<u64> {
    const DAY: u64 = 86_400;
    match (p, kind) {
        (ProviderId::Farnell, _) => None,
        (ProviderId::Tme, "params") => Some(30 * DAY),
        _ => Some(DAY),
    }
}

pub struct Cache {
    db: Mutex<rusqlite::Connection>,
}

impl Cache {
    pub fn open(path: &std::path::Path) -> Result<Self, String> {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        Self::with(rusqlite::Connection::open(path).map_err(|e| e.to_string())?)
    }
    pub fn memory() -> Self {
        Self::with(rusqlite::Connection::open_in_memory().expect("sqlite")).expect("cache")
    }
    fn with(db: rusqlite::Connection) -> Result<Self, String> {
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS cache (provider TEXT NOT NULL, kind TEXT NOT NULL, key TEXT NOT NULL, value TEXT NOT NULL, fetched INTEGER NOT NULL, PRIMARY KEY (provider, kind, key));",
        )
        .map_err(|e| e.to_string())?;
        Ok(Self { db: Mutex::new(db) })
    }

    /// A fresh entry, if any.
    pub fn get(&self, p: ProviderId, kind: &str, key: &str) -> Option<String> {
        let ttl = lifetime(p, kind)?;
        let db = self.db.lock().ok()?;
        let row: Option<(String, i64)> = db
            .query_row(
                "SELECT value, fetched FROM cache WHERE provider = ?1 AND kind = ?2 AND key = ?3",
                (p.key(), kind, key),
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .ok();
        let (value, fetched) = row?;
        (crate::now().saturating_sub(fetched as u64) <= ttl).then_some(value)
    }

    pub fn put(&self, p: ProviderId, kind: &str, key: &str, value: &str) {
        if lifetime(p, kind).is_none() {
            return;
        }
        if let Ok(db) = self.db.lock() {
            let _ = db.execute(
                "INSERT OR REPLACE INTO cache (provider, kind, key, value, fetched) VALUES (?1, ?2, ?3, ?4, ?5)",
                (p.key(), kind, key, value, crate::now() as i64),
            );
        }
    }

    /// Delete a provider's cache (Settings › Online catalogs).
    pub fn clear(&self, p: ProviderId) {
        if let Ok(db) = self.db.lock() {
            let _ = db.execute("DELETE FROM cache WHERE provider = ?1", [p.key()]);
        }
    }

    /// Drop expired entries.
    pub fn prune(&self) {
        if let Ok(db) = self.db.lock() {
            for p in ProviderId::ALL {
                for kind in ["search", "part", "params"] {
                    let ttl = lifetime(p, kind).unwrap_or(0) as i64;
                    let _ = db.execute(
                        "DELETE FROM cache WHERE provider = ?1 AND kind = ?2 AND fetched < ?3",
                        (p.key(), kind, crate::now() as i64 - ttl),
                    );
                }
            }
        }
    }

    pub fn entries(&self, p: ProviderId) -> usize {
        self.db
            .lock()
            .ok()
            .and_then(|db| {
                db.query_row(
                    "SELECT COUNT(*) FROM cache WHERE provider = ?1",
                    [p.key()],
                    |r| r.get::<_, i64>(0),
                )
                .ok()
            })
            .unwrap_or(0) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_fetch_clear_and_farnell_never() {
        let c = Cache::memory();
        c.put(ProviderId::Digikey, "search", "k", "v");
        assert_eq!(
            c.get(ProviderId::Digikey, "search", "k").as_deref(),
            Some("v")
        );
        c.put(ProviderId::Farnell, "search", "k", "v");
        assert_eq!(c.get(ProviderId::Farnell, "search", "k"), None);
        assert_eq!(c.entries(ProviderId::Farnell), 0);
        c.clear(ProviderId::Digikey);
        assert_eq!(c.get(ProviderId::Digikey, "search", "k"), None);
    }
}

//! Credentials in the operating system keychain (the `keyring` crate):
//! never in plain files or schematics. Entries are named
//! `electrocraft-catalog` / `<provider>.<field>`.

use crate::ProviderId;
use std::collections::BTreeMap;
use std::sync::Mutex;

const SERVICE: &str = "electrocraft-catalog";

/// Where credentials are kept.
pub trait Store: Send + Sync {
    fn get(&self, p: ProviderId, field: &str) -> Option<String>;
    fn set(&self, p: ProviderId, field: &str, value: &str) -> Result<(), String>;
    fn delete(&self, p: ProviderId, field: &str) -> Result<(), String>;
    /// Every field the provider needs is present.
    fn complete(&self, p: ProviderId) -> bool {
        p.credential_fields()
            .iter()
            .all(|(f, _)| self.get(p, f).is_some_and(|v| !v.trim().is_empty()))
    }
}

/// The system keychain (Secret Service on Linux, Keychain on macOS,
/// Credential Manager on Windows).
pub struct Keychain;

fn entry(p: ProviderId, field: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new(SERVICE, &format!("{}.{field}", p.key())).map_err(|e| e.to_string())
}

impl Store for Keychain {
    fn get(&self, p: ProviderId, field: &str) -> Option<String> {
        entry(p, field).ok()?.get_password().ok()
    }
    fn set(&self, p: ProviderId, field: &str, value: &str) -> Result<(), String> {
        entry(p, field)?
            .set_password(value)
            .map_err(|e| format!("keychain: {e}"))
    }
    fn delete(&self, p: ProviderId, field: &str) -> Result<(), String> {
        match entry(p, field)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(format!("keychain: {e}")),
        }
    }
}

/// In memory (tests, or when no keychain is available).
#[derive(Default)]
pub struct Memory {
    map: Mutex<BTreeMap<String, String>>,
}

impl Store for Memory {
    fn get(&self, p: ProviderId, field: &str) -> Option<String> {
        self.map
            .lock()
            .ok()?
            .get(&format!("{}.{field}", p.key()))
            .cloned()
    }
    fn set(&self, p: ProviderId, field: &str, value: &str) -> Result<(), String> {
        self.map
            .lock()
            .map_err(|e| e.to_string())?
            .insert(format!("{}.{field}", p.key()), value.into());
        Ok(())
    }
    fn delete(&self, p: ProviderId, field: &str) -> Result<(), String> {
        self.map
            .lock()
            .map_err(|e| e.to_string())?
            .remove(&format!("{}.{field}", p.key()));
        Ok(())
    }
}

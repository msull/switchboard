//! Secret values by account name. The records hold only names; the
//! values live here (the macOS Keychain in the app, memory in tests).

pub trait SecretStore: Send + Sync {
    /// `Ok(None)` when no such item exists.
    fn get(&self, account: &str) -> Result<Option<String>, String>;
    fn set(&self, account: &str, value: &str) -> Result<(), String>;
    /// Deleting a missing item is not an error.
    fn delete(&self, account: &str) -> Result<(), String>;
}

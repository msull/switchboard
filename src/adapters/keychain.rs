//! Secrets in the macOS Keychain as generic-password items under one
//! service name, so they are auditable in Keychain Access. Spike 5 showed
//! a bundle signed with a stable identity reads its own items across
//! rebuilds without a prompt. Tests use a throwaway keychain file.

use security_framework::os::macos::keychain::{CreateOptions, SecKeychain};
use security_framework::passwords;

use crate::ports::secrets::SecretStore;

/// The Keychain service every Switchboard item is filed under.
pub const SERVICE: &str = "com.sadburger.switchboard";

pub struct KeychainStore {
    /// `None` means the user's default (login) keychain.
    keychain: Option<SecKeychain>,
}

impl KeychainStore {
    /// The login keychain.
    #[must_use]
    pub fn login() -> Self {
        Self { keychain: None }
    }

    /// A fresh keychain file at `path`, unlocked; for tests.
    ///
    /// # Errors
    /// The file cannot be created.
    pub fn create_at(path: &std::path::Path) -> Result<Self, String> {
        let keychain = CreateOptions::new()
            .password("switchboard-test")
            .create(path)
            .map_err(|e| e.to_string())?;
        Ok(Self {
            keychain: Some(keychain),
        })
    }
}

/// `errSecItemNotFound`.
const NOT_FOUND: i32 = -25300;

fn not_found(e: security_framework::base::Error) -> bool {
    e.code() == NOT_FOUND
}

impl SecretStore for KeychainStore {
    fn get(&self, account: &str) -> Result<Option<String>, String> {
        let bytes = match &self.keychain {
            None => passwords::get_generic_password(SERVICE, account),
            Some(k) => k
                .find_generic_password(SERVICE, account)
                .map(|(pw, _)| pw.to_vec()),
        };
        match bytes {
            Ok(bytes) => Ok(Some(String::from_utf8_lossy(&bytes).into_owned())),
            Err(e) if not_found(e) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    fn set(&self, account: &str, value: &str) -> Result<(), String> {
        match &self.keychain {
            None => passwords::set_generic_password(SERVICE, account, value.as_bytes()),
            Some(k) => k.set_generic_password(SERVICE, account, value.as_bytes()),
        }
        .map_err(|e| e.to_string())
    }

    fn delete(&self, account: &str) -> Result<(), String> {
        let result = match &self.keychain {
            None => passwords::delete_generic_password(SERVICE, account),
            Some(k) => k.find_generic_password(SERVICE, account).map(|(_, item)| {
                item.delete();
            }),
        };
        match result {
            Ok(()) => Ok(()),
            Err(e) if not_found(e) => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_in_a_throwaway_keychain() {
        let dir = tempfile::tempdir().unwrap();
        let store = KeychainStore::create_at(&dir.path().join("test.keychain-db")).unwrap();
        assert_eq!(store.get("global/NOPE").unwrap(), None);
        store.set("global/TOKEN", "s3cret").unwrap();
        assert_eq!(
            store.get("global/TOKEN").unwrap().as_deref(),
            Some("s3cret")
        );
        store.set("global/TOKEN", "changed").unwrap();
        assert_eq!(
            store.get("global/TOKEN").unwrap().as_deref(),
            Some("changed")
        );
        store.delete("global/TOKEN").unwrap();
        store.delete("global/TOKEN").unwrap();
        assert_eq!(store.get("global/TOKEN").unwrap(), None);
    }
}

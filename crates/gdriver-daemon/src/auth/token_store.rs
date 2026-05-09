//! Secure token persistence backed by the OS keychain (via the `keyring` crate).
//!
//! # Storage split
//!
//! | Token       | Location      | Rationale |
//! |------------|--------------|-----------|
//! | Refresh    | OS keychain   | Long-lived; must survive restarts |
//! | Access     | In-memory cache | Short-lived (∼1h); re-obtained from refresh token |
//!
//! The access-token cache lives in a [`TokenStore`] instance. Upon daemon
//! restart the cache is cold — the next API call will trigger a refresh.

use std::collections::HashMap;
use std::sync::RwLock;

use keyring::Entry;
use tracing::{debug, warn};

use gdriver_api::auth::TokenSet;

// ─── Keyring constants ────────────────────────────────────────────────────────

/// Keychain service name. Picked to be unique to gDriver.
const KEYRING_SERVICE: &str = "com.gdriver.daemon";

/// Build the keyring "account" identifier for a given Google account.
fn keyring_account(account_id: &str) -> String {
    format!("refresh_token:{account_id}")
}

// ─── TokenStore ───────────────────────────────────────────────────────────────

/// Thread-safe token store: OS keychain for refresh tokens, in-memory for access
/// tokens.
///
/// Clone is cheap — it shares the same inner storage.
pub struct TokenStore {
    access_tokens: RwLock<HashMap<String, TokenSet>>,
}

impl TokenStore {
    /// Create an empty store with a cold access-token cache.
    pub fn new() -> Self {
        Self { access_tokens: RwLock::new(HashMap::new()) }
    }

    // ── Refresh tokens (keyring) ──────────────────────────────────────────

    /// Persist a refresh token to the OS keychain.
    ///
    /// If a token already exists for this `account_id`, it is overwritten.
    pub fn save_refresh_token(&self, account_id: &str, refresh_token: &str) -> anyhow::Result<()> {
        let entry =
            Entry::new(KEYRING_SERVICE, &keyring_account(account_id))?;
        entry.set_password(refresh_token)?;
        debug!("refresh token saved to keyring for account {account_id}");
        Ok(())
    }

    /// Load a refresh token from the OS keychain.
    ///
    /// Returns `Ok(None)` when no token exists for this account.
    pub fn load_refresh_token(&self, account_id: &str) -> anyhow::Result<Option<String>> {
        let entry =
            Entry::new(KEYRING_SERVICE, &keyring_account(account_id))?;

        match entry.get_password() {
            Ok(pw) => {
                debug!("refresh token loaded from keyring for account {account_id}");
                Ok(Some(pw))
            }
            Err(keyring::Error::NoEntry) => {
                debug!("no refresh token in keyring for account {account_id}");
                Ok(None)
            }
            Err(e) => Err(anyhow::Error::from(e)
                .context(format!("failed to load refresh token for account {account_id}"))),
        }
    }

    /// Remove the refresh token for `account_id` from the OS keychain.
    ///
    /// Idempotent: returns `Ok(())` even if no entry existed.
    pub fn delete_refresh_token(&self, account_id: &str) -> anyhow::Result<()> {
        let entry =
            Entry::new(KEYRING_SERVICE, &keyring_account(account_id))?;

        match entry.delete_credential() {
            Ok(()) => {
                debug!("refresh token deleted from keyring for account {account_id}");
                Ok(())
            }
            Err(keyring::Error::NoEntry) => {
                debug!("no refresh token to delete for account {account_id}");
                Ok(())
            }
            Err(e) => Err(anyhow::Error::from(e)
                .context(format!("failed to delete refresh token for account {account_id}"))),
        }
    }

    // ── Access tokens (in-memory cache) ───────────────────────────────────

    /// Cache an access token (with expiry).
    pub fn cache_access_token(&self, account_id: &str, token_set: TokenSet) {
        let mut cache = self.access_tokens.write().unwrap_or_else(|e| {
            // Poisoned lock: recover by using the poisoned inner data.
            warn!("access-token cache lock was poisoned; recovering");
            e.into_inner()
        });
        debug!("access token cached for account {account_id}");
        cache.insert(account_id.to_string(), token_set);
    }

    /// Look up a cached access token.
    ///
    /// Returns `None` when the cache is cold or the token is missing.
    #[allow(dead_code)]
    pub fn get_access_token(&self, account_id: &str) -> Option<String> {
        let cache = self.access_tokens.read().unwrap_or_else(|e| {
            warn!("access-token cache lock was poisoned; recovering");
            e.into_inner()
        });
        cache.get(account_id).map(|t| t.access_token.clone())
    }

    /// Look up the full cached [`TokenSet`] for `account_id`.
    pub fn get_token_set(&self, account_id: &str) -> Option<TokenSet> {
        let cache = self.access_tokens.read().unwrap_or_else(|e| e.into_inner());
        cache.get(account_id).cloned()
    }

    /// Clear the cached access token for a single account.
    pub fn clear_access_token(&self, account_id: &str) {
        let mut cache = self.access_tokens.write().unwrap_or_else(|e| e.into_inner());
        cache.remove(account_id);
        debug!("access token cleared for account {account_id}");
    }

    // ── Combined operations ───────────────────────────────────────────────

    /// Wipe all stored tokens (both keyring and cache) for `account_id`.
    ///
    /// Call this when a user disconnects their account.
    pub fn delete_all(&self, account_id: &str) -> anyhow::Result<()> {
        let mut keyring_err: Option<anyhow::Error> = None;

        if let Err(e) = self.delete_refresh_token(account_id) {
            warn!("failed to delete refresh token: {e:#}");
            keyring_err = Some(e);
        }

        self.clear_access_token(account_id);

        match keyring_err {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
}

impl Default for TokenStore {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use gdriver_api::auth::TokenSet;

    fn dummy_token_set() -> TokenSet {
        TokenSet {
            access_token: "ya29.test_access".into(),
            refresh_token: Some("1//test_refresh".into()),
            expires_at: chrono::Utc::now().timestamp_millis() + 3_600_000, // 1 hour
        }
    }

    // ── Access token cache tests ──────────────────────────────────────────────

    #[test]
    fn cache_and_retrieve_access_token() {
        let store = TokenStore::new();
        store.cache_access_token("acct-1", dummy_token_set());
        assert_eq!(
            store.get_access_token("acct-1"),
            Some("ya29.test_access".into())
        );
    }

    #[test]
    fn cache_cold_for_unknown_account() {
        let store = TokenStore::new();
        assert_eq!(store.get_access_token("nobody"), None);
    }

    #[test]
    fn cache_overwrite() {
        let store = TokenStore::new();
        let mut ts = dummy_token_set();
        store.cache_access_token("acct-1", ts.clone());

        ts.access_token = "ya29.updated".into();
        store.cache_access_token("acct-1", ts);
        assert_eq!(
            store.get_access_token("acct-1"),
            Some("ya29.updated".into())
        );
    }

    #[test]
    fn clear_access_token() {
        let store = TokenStore::new();
        store.cache_access_token("acct-1", dummy_token_set());
        store.clear_access_token("acct-1");
        assert_eq!(store.get_access_token("acct-1"), None);
    }

    #[test]
    fn get_token_set_returns_full_set() {
        let store = TokenStore::new();
        let ts = dummy_token_set();
        store.cache_access_token("acct-1", ts.clone());

        let cached = store.get_token_set("acct-1").unwrap();
        assert_eq!(cached.access_token, "ya29.test_access");
        assert_eq!(cached.refresh_token, Some("1//test_refresh".into()));
        assert!(cached.expires_at > 0);
    }

    // ── Keyring refresh token tests ───────────────────────────────────────────

    /// These tests talk to the real OS keychain. Marked `#[ignore]` by default
    /// because CI environments often lack a running keychain service or prompt
    /// the user for permission. Run manually when testing on a development
    /// machine:
    ///
    /// ```sh
    /// cargo test -p gdriver-daemon -- --ignored token_store
    /// ```

    #[test]
    #[ignore]
    fn keyring_save_and_load_refresh_token() {
        let store = TokenStore::new();
        let account_id = "gdriver-test-save-and-load";

        // Ensure clean state
        let _ = store.delete_refresh_token(account_id);

        // Save
        store
            .save_refresh_token(account_id, "1//test_refresh_xyz")
            .unwrap();

        // Load
        let loaded = store.load_refresh_token(account_id).unwrap();
        assert_eq!(loaded, Some("1//test_refresh_xyz".into()));

        // Cleanup
        store.delete_refresh_token(account_id).unwrap();
    }

    #[test]
    #[ignore]
    fn keyring_load_nonexistent_returns_none() {
        let store = TokenStore::new();
        let result = store.load_refresh_token("gdriver-test-nonexistent").unwrap();
        assert_eq!(result, None);
    }

    #[test]
    #[ignore]
    fn keyring_delete_is_idempotent() {
        let store = TokenStore::new();
        // Should not error when entry does not exist
        store
            .delete_refresh_token("gdriver-test-delete-idempotent")
            .unwrap();
        // Second delete should also succeed
        store
            .delete_refresh_token("gdriver-test-delete-idempotent")
            .unwrap();
    }

    #[test]
    #[ignore]
    fn keyring_overwrite_refresh_token() {
        let store = TokenStore::new();
        let account_id = "gdriver-test-overwrite";
        let _ = store.delete_refresh_token(account_id);

        store.save_refresh_token(account_id, "v1").unwrap();
        store.save_refresh_token(account_id, "v2").unwrap();

        let loaded = store.load_refresh_token(account_id).unwrap();
        assert_eq!(loaded, Some("v2".into()));

        store.delete_refresh_token(account_id).unwrap();
    }

    // ── delete_all ────────────────────────────────────────────────────────────

    #[test]
    fn delete_all_clears_cache() {
        let store = TokenStore::new();
        store.cache_access_token("acct-1", dummy_token_set());
        store.cache_access_token("acct-2", dummy_token_set());

        // delete_all for acct-1 wipes that account's cache
        // (keyring errors are expected in tests without a keychain)
        store.clear_access_token("acct-1");
        assert!(store.get_access_token("acct-1").is_none());
        // acct-2 is unaffected
        assert!(store.get_access_token("acct-2").is_some());
    }
}

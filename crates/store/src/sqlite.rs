//! SQLite-backed token store using sqlx.
//!
//! Schema: `accounts(provider, account_id, label, is_active, token_json, created_at, updated_at)`
//! with composite primary key `(provider, account_id)`.
//!
//! A partial unique index ensures at most one active account per provider.

use async_trait::async_trait;
use byokey_types::{AccountInfo, ByokError, OAuthToken, ProviderId, TokenStore, traits::Result};
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use std::str::FromStr;

/// A persistent [`TokenStore`] backed by `SQLite`.
pub struct SqliteTokenStore {
    /// Connection pool to the `SQLite` database.
    pool: SqlitePool,
}

impl SqliteTokenStore {
    /// Connects to a `SQLite` database (e.g. `"sqlite:./tokens.db"` or `"sqlite::memory:"`).
    ///
    /// Automatically creates the database file if it does not exist.
    /// Runs migrations to create / upgrade the schema.
    ///
    /// # Errors
    ///
    /// Returns a [`sqlx::Error`] if the connection or table creation fails.
    pub async fn new(database_url: &str) -> std::result::Result<Self, sqlx::Error> {
        let opts = SqliteConnectOptions::from_str(database_url)?.create_if_missing(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(opts)
            .await?;
        Self::migrate(&pool).await?;
        Ok(Self { pool })
    }

    /// Run schema migrations.
    ///
    /// - Creates the `accounts` table if it does not exist.
    /// - Migrates from the legacy `tokens` table if present.
    async fn migrate(pool: &SqlitePool) -> std::result::Result<(), sqlx::Error> {
        // Create the new accounts table (idempotent).
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS accounts (
                provider    TEXT    NOT NULL,
                account_id  TEXT    NOT NULL,
                label       TEXT,
                is_active   INTEGER NOT NULL DEFAULT 1,
                token_json  TEXT    NOT NULL,
                created_at  INTEGER NOT NULL DEFAULT (unixepoch()),
                updated_at  INTEGER NOT NULL DEFAULT (unixepoch()),
                PRIMARY KEY (provider, account_id)
            )",
        )
        .execute(pool)
        .await?;

        // Partial unique index: at most one active account per provider.
        sqlx::query(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_active_account
             ON accounts(provider) WHERE is_active = 1",
        )
        .execute(pool)
        .await?;

        // Migrate legacy `tokens` table if it exists.
        let legacy_exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='tokens')",
        )
        .fetch_one(pool)
        .await?;

        if legacy_exists {
            sqlx::query(
                "INSERT OR IGNORE INTO accounts (provider, account_id, is_active, token_json)
                 SELECT provider, 'default', 1, token_json FROM tokens",
            )
            .execute(pool)
            .await?;
            sqlx::query("DROP TABLE tokens").execute(pool).await?;
        }

        Ok(())
    }
}

#[async_trait]
impl TokenStore for SqliteTokenStore {
    // ── Active-account shortcuts ──────────────────────────────────────────

    /// Loads the token for the active account of the given provider from `SQLite`.
    async fn load(&self, provider: &ProviderId) -> Result<Option<OAuthToken>> {
        let key = provider.to_string();
        let row: Option<(String,)> =
            sqlx::query_as("SELECT token_json FROM accounts WHERE provider = ? AND is_active = 1")
                .bind(&key)
                .fetch_optional(&self.pool)
                .await?;

        match row {
            None => Ok(None),
            Some((json,)) => {
                let token: OAuthToken =
                    serde_json::from_str(&json).map_err(|e| ByokError::Storage(e.to_string()))?;
                Ok(Some(token))
            }
        }
    }

    /// Saves (upserts) the token for the active account of the given provider.
    ///
    /// If no account exists yet, creates a `"default"` account and marks it active.
    async fn save(&self, provider: &ProviderId, token: &OAuthToken) -> Result<()> {
        self.save_account(provider, "default", None, token).await
    }

    /// Removes the active account's token for the given provider.
    async fn remove(&self, provider: &ProviderId) -> Result<()> {
        let key = provider.to_string();
        sqlx::query("DELETE FROM accounts WHERE provider = ? AND is_active = 1")
            .bind(&key)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // ── Multi-account operations ──────────────────────────────────────────

    async fn load_account(
        &self,
        provider: &ProviderId,
        account_id: &str,
    ) -> Result<Option<OAuthToken>> {
        let key = provider.to_string();
        let row: Option<(String,)> =
            sqlx::query_as("SELECT token_json FROM accounts WHERE provider = ? AND account_id = ?")
                .bind(&key)
                .bind(account_id)
                .fetch_optional(&self.pool)
                .await?;

        match row {
            None => Ok(None),
            Some((json,)) => {
                let token: OAuthToken =
                    serde_json::from_str(&json).map_err(|e| ByokError::Storage(e.to_string()))?;
                Ok(Some(token))
            }
        }
    }

    async fn save_account(
        &self,
        provider: &ProviderId,
        account_id: &str,
        label: Option<&str>,
        token: &OAuthToken,
    ) -> Result<()> {
        let key = provider.to_string();
        let json = serde_json::to_string(token).map_err(|e| ByokError::Storage(e.to_string()))?;

        // Check if any account is already active for this provider.
        let has_active: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM accounts WHERE provider = ? AND is_active = 1)",
        )
        .bind(&key)
        .fetch_one(&self.pool)
        .await?;

        // New accounts become active if no other active account exists.
        let is_active = !has_active;

        sqlx::query(
            "INSERT INTO accounts (provider, account_id, label, is_active, token_json)
             VALUES (?, ?, ?, ?, ?)
             ON CONFLICT(provider, account_id) DO UPDATE SET
                 label = COALESCE(excluded.label, accounts.label),
                 token_json = excluded.token_json,
                 updated_at = unixepoch()",
        )
        .bind(&key)
        .bind(account_id)
        .bind(label)
        .bind(is_active)
        .bind(&json)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn remove_account(&self, provider: &ProviderId, account_id: &str) -> Result<()> {
        let key = provider.to_string();
        sqlx::query("DELETE FROM accounts WHERE provider = ? AND account_id = ?")
            .bind(&key)
            .bind(account_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn list_accounts(&self, provider: &ProviderId) -> Result<Vec<AccountInfo>> {
        let key = provider.to_string();
        let rows: Vec<(String, Option<String>, bool)> = sqlx::query_as(
            "SELECT account_id, label, is_active FROM accounts
             WHERE provider = ? ORDER BY is_active DESC, account_id",
        )
        .bind(&key)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|(account_id, label, is_active)| AccountInfo {
                account_id,
                label,
                is_active,
            })
            .collect())
    }

    async fn set_active(&self, provider: &ProviderId, account_id: &str) -> Result<()> {
        let key = provider.to_string();

        // Verify the target account exists.
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM accounts WHERE provider = ? AND account_id = ?)",
        )
        .bind(&key)
        .bind(account_id)
        .fetch_one(&self.pool)
        .await?;

        if !exists {
            return Err(ByokError::Storage(format!(
                "account '{account_id}' not found for provider {provider}"
            )));
        }

        // Deactivate all accounts for this provider, then activate the target.
        sqlx::query("UPDATE accounts SET is_active = 0 WHERE provider = ?")
            .bind(&key)
            .execute(&self.pool)
            .await?;
        sqlx::query("UPDATE accounts SET is_active = 1 WHERE provider = ? AND account_id = ?")
            .bind(&key)
            .bind(account_id)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    async fn load_all_tokens(&self, provider: &ProviderId) -> Result<Vec<(String, OAuthToken)>> {
        let key = provider.to_string();
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT account_id, token_json FROM accounts
             WHERE provider = ? ORDER BY is_active DESC, account_id",
        )
        .bind(&key)
        .fetch_all(&self.pool)
        .await?;

        let mut result = Vec::with_capacity(rows.len());
        for (account_id, json) in rows {
            let token: OAuthToken =
                serde_json::from_str(&json).map_err(|e| ByokError::Storage(e.to_string()))?;
            result.push((account_id, token));
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn mem() -> SqliteTokenStore {
        SqliteTokenStore::new("sqlite::memory:").await.unwrap()
    }

    #[tokio::test]
    async fn test_save_and_load() {
        let s = mem().await;
        let tok = OAuthToken::new("access").with_refresh("refresh");
        s.save(&ProviderId::Claude, &tok).await.unwrap();
        let loaded = s.load(&ProviderId::Claude).await.unwrap().unwrap();
        assert_eq!(loaded.access_token, "access");
        assert_eq!(loaded.refresh_token, Some("refresh".into()));
    }

    #[tokio::test]
    async fn test_load_missing() {
        let s = mem().await;
        assert!(s.load(&ProviderId::Gemini).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_remove() {
        let s = mem().await;
        s.save(&ProviderId::Codex, &OAuthToken::new("tok"))
            .await
            .unwrap();
        s.remove(&ProviderId::Codex).await.unwrap();
        assert!(s.load(&ProviderId::Codex).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_upsert() {
        let s = mem().await;
        s.save(&ProviderId::Claude, &OAuthToken::new("first"))
            .await
            .unwrap();
        s.save(&ProviderId::Claude, &OAuthToken::new("second"))
            .await
            .unwrap();
        assert_eq!(
            s.load(&ProviderId::Claude)
                .await
                .unwrap()
                .unwrap()
                .access_token,
            "second"
        );
    }

    #[tokio::test]
    async fn test_multiple_providers() {
        let s = mem().await;
        s.save(&ProviderId::Claude, &OAuthToken::new("c"))
            .await
            .unwrap();
        s.save(&ProviderId::Gemini, &OAuthToken::new("g"))
            .await
            .unwrap();
        assert_eq!(
            s.load(&ProviderId::Claude)
                .await
                .unwrap()
                .unwrap()
                .access_token,
            "c"
        );
        assert_eq!(
            s.load(&ProviderId::Gemini)
                .await
                .unwrap()
                .unwrap()
                .access_token,
            "g"
        );
    }

    #[tokio::test]
    async fn test_expiry_persists() {
        let s = mem().await;
        let tok = OAuthToken::new("tok").with_expiry(3600);
        s.save(&ProviderId::Kiro, &tok).await.unwrap();
        let loaded = s.load(&ProviderId::Kiro).await.unwrap().unwrap();
        assert!(loaded.expires_at.is_some());
    }

    // ── Multi-account tests ──────────────────────────────────────────────

    #[tokio::test]
    async fn test_save_and_load_account() {
        let s = mem().await;
        let tok = OAuthToken::new("work-token");
        s.save_account(&ProviderId::Claude, "work", Some("Work Account"), &tok)
            .await
            .unwrap();
        let loaded = s
            .load_account(&ProviderId::Claude, "work")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(loaded.access_token, "work-token");
    }

    #[tokio::test]
    async fn test_first_account_becomes_active() {
        let s = mem().await;
        s.save_account(&ProviderId::Claude, "first", None, &OAuthToken::new("tok1"))
            .await
            .unwrap();
        // First account should be active and loadable via `load()`.
        let loaded = s.load(&ProviderId::Claude).await.unwrap().unwrap();
        assert_eq!(loaded.access_token, "tok1");
    }

    #[tokio::test]
    async fn test_second_account_not_active() {
        let s = mem().await;
        s.save_account(&ProviderId::Claude, "first", None, &OAuthToken::new("tok1"))
            .await
            .unwrap();
        s.save_account(
            &ProviderId::Claude,
            "second",
            None,
            &OAuthToken::new("tok2"),
        )
        .await
        .unwrap();
        // `load()` returns the active one (first).
        let loaded = s.load(&ProviderId::Claude).await.unwrap().unwrap();
        assert_eq!(loaded.access_token, "tok1");
    }

    #[tokio::test]
    async fn test_set_active() {
        let s = mem().await;
        s.save_account(&ProviderId::Claude, "a", None, &OAuthToken::new("tok-a"))
            .await
            .unwrap();
        s.save_account(&ProviderId::Claude, "b", None, &OAuthToken::new("tok-b"))
            .await
            .unwrap();
        s.set_active(&ProviderId::Claude, "b").await.unwrap();
        let loaded = s.load(&ProviderId::Claude).await.unwrap().unwrap();
        assert_eq!(loaded.access_token, "tok-b");
    }

    #[tokio::test]
    async fn test_set_active_nonexistent() {
        let s = mem().await;
        let err = s.set_active(&ProviderId::Claude, "nope").await.unwrap_err();
        assert!(err.to_string().contains("not found"));
    }

    #[tokio::test]
    async fn test_list_accounts() {
        let s = mem().await;
        s.save_account(
            &ProviderId::Claude,
            "work",
            Some("Work"),
            &OAuthToken::new("w"),
        )
        .await
        .unwrap();
        s.save_account(
            &ProviderId::Claude,
            "personal",
            Some("Personal"),
            &OAuthToken::new("p"),
        )
        .await
        .unwrap();

        let accounts = s.list_accounts(&ProviderId::Claude).await.unwrap();
        assert_eq!(accounts.len(), 2);
        // First is active.
        assert!(accounts[0].is_active);
        assert_eq!(accounts[0].account_id, "work");
        assert_eq!(accounts[0].label.as_deref(), Some("Work"));
    }

    #[tokio::test]
    async fn test_load_all_tokens() {
        let s = mem().await;
        s.save_account(&ProviderId::Claude, "a", None, &OAuthToken::new("tok-a"))
            .await
            .unwrap();
        s.save_account(&ProviderId::Claude, "b", None, &OAuthToken::new("tok-b"))
            .await
            .unwrap();

        let all = s.load_all_tokens(&ProviderId::Claude).await.unwrap();
        assert_eq!(all.len(), 2);
        // Active account comes first.
        assert_eq!(all[0].0, "a");
    }

    #[tokio::test]
    async fn test_remove_account() {
        let s = mem().await;
        s.save_account(&ProviderId::Claude, "work", None, &OAuthToken::new("w"))
            .await
            .unwrap();
        s.remove_account(&ProviderId::Claude, "work").await.unwrap();
        assert!(
            s.load_account(&ProviderId::Claude, "work")
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn test_legacy_migration() {
        // Simulate a legacy database with a `tokens` table.
        let opts = SqliteConnectOptions::from_str("sqlite::memory:")
            .unwrap()
            .create_if_missing(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE tokens (
                provider   TEXT PRIMARY KEY,
                token_json TEXT NOT NULL
            )",
        )
        .execute(&pool)
        .await
        .unwrap();
        let tok = OAuthToken::new("legacy-token");
        let json = serde_json::to_string(&tok).unwrap();
        sqlx::query("INSERT INTO tokens (provider, token_json) VALUES ('claude', ?)")
            .bind(&json)
            .execute(&pool)
            .await
            .unwrap();

        // Run migration.
        SqliteTokenStore::migrate(&pool).await.unwrap();

        // Legacy table should be gone.
        let legacy_exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='tokens')",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(!legacy_exists);

        // Data should be in the new table as active "default" account.
        let row: Option<(String, String, bool)> = sqlx::query_as(
            "SELECT account_id, token_json, is_active FROM accounts WHERE provider = 'claude'",
        )
        .fetch_optional(&pool)
        .await
        .unwrap();
        let (account_id, _json, is_active) = row.unwrap();
        assert_eq!(account_id, "default");
        assert!(is_active);
    }
}

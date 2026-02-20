//! SQLite-backed token store using sqlx.

use async_trait::async_trait;
use byokey_types::{ByokError, OAuthToken, ProviderId, TokenStore, traits::Result};
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
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS tokens (
                provider   TEXT PRIMARY KEY,
                token_json TEXT NOT NULL
            )",
        )
        .execute(&pool)
        .await?;
        Ok(Self { pool })
    }
}

#[async_trait]
impl TokenStore for SqliteTokenStore {
    /// Loads the token for the given provider from `SQLite`.
    async fn load(&self, provider: &ProviderId) -> Result<Option<OAuthToken>> {
        let key = provider.to_string();
        let row: Option<(String,)> =
            sqlx::query_as("SELECT token_json FROM tokens WHERE provider = ?")
                .bind(&key)
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| ByokError::Storage(e.to_string()))?;

        match row {
            None => Ok(None),
            Some((json,)) => {
                let token: OAuthToken =
                    serde_json::from_str(&json).map_err(|e| ByokError::Storage(e.to_string()))?;
                Ok(Some(token))
            }
        }
    }

    /// Saves (upserts) the token for the given provider into `SQLite`.
    async fn save(&self, provider: &ProviderId, token: &OAuthToken) -> Result<()> {
        let key = provider.to_string();
        let json = serde_json::to_string(token).map_err(|e| ByokError::Storage(e.to_string()))?;
        sqlx::query(
            "INSERT INTO tokens (provider, token_json) VALUES (?, ?)
             ON CONFLICT(provider) DO UPDATE SET token_json = excluded.token_json",
        )
        .bind(&key)
        .bind(&json)
        .execute(&self.pool)
        .await
        .map_err(|e| ByokError::Storage(e.to_string()))?;
        Ok(())
    }

    /// Removes the token for the given provider from `SQLite`.
    async fn remove(&self, provider: &ProviderId) -> Result<()> {
        let key = provider.to_string();
        sqlx::query("DELETE FROM tokens WHERE provider = ?")
            .bind(&key)
            .execute(&self.pool)
            .await
            .map_err(|e| ByokError::Storage(e.to_string()))?;
        Ok(())
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
}

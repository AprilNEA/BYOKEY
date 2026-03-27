//! [`TokenStore`] implementation for [`SqliteTokenStore`].

use async_trait::async_trait;
use byokey_types::{AccountInfo, ByokError, OAuthToken, ProviderId, TokenStore, traits::Result};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, Set};

use super::{SqliteTokenStore, db_exec_raw, now_unix};
use crate::entity::account;

#[async_trait]
impl TokenStore for SqliteTokenStore {
    // ── Active-account shortcuts ──────────────────────────────────────────

    /// Loads the token for the active account of the given provider from `SQLite`.
    async fn load(&self, provider: &ProviderId) -> Result<Option<OAuthToken>> {
        let key = provider.to_string();
        let row = account::Entity::find()
            .filter(account::Column::Provider.eq(&key))
            .filter(account::Column::IsActive.eq(true))
            .one(&self.db)
            .await?;

        match row {
            None => Ok(None),
            Some(m) => {
                let token: OAuthToken = serde_json::from_str(&m.token_json)
                    .map_err(|e| ByokError::Storage(e.to_string()))?;
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
        account::Entity::delete_many()
            .filter(account::Column::Provider.eq(&key))
            .filter(account::Column::IsActive.eq(true))
            .exec(&self.db)
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
        let row = account::Entity::find_by_id((key, account_id.to_string()))
            .one(&self.db)
            .await?;

        match row {
            None => Ok(None),
            Some(m) => {
                let token: OAuthToken = serde_json::from_str(&m.token_json)
                    .map_err(|e| ByokError::Storage(e.to_string()))?;
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
        let now = now_unix();

        // Check if any account is already active for this provider.
        let has_active = account::Entity::find()
            .filter(account::Column::Provider.eq(&key))
            .filter(account::Column::IsActive.eq(true))
            .one(&self.db)
            .await?
            .is_some();

        // Check if this specific account already exists.
        let existing = account::Entity::find_by_id((key.clone(), account_id.to_string()))
            .one(&self.db)
            .await?;

        if let Some(existing_model) = existing {
            // Update existing account.
            let mut active: account::ActiveModel = existing_model.into();
            active.token_json = Set(json);
            active.updated_at = Set(now);
            if let Some(l) = label {
                active.label = Set(Some(l.to_string()));
            }
            active.update(&self.db).await?;
        } else {
            // New accounts become active if no other active account exists.
            let is_active = !has_active;
            let model = account::ActiveModel {
                provider: Set(key),
                account_id: Set(account_id.to_string()),
                label: Set(label.map(String::from)),
                is_active: Set(is_active),
                token_json: Set(json),
                created_at: Set(now),
                updated_at: Set(now),
            };
            model.insert(&self.db).await?;
        }

        Ok(())
    }

    async fn remove_account(&self, provider: &ProviderId, account_id: &str) -> Result<()> {
        let key = provider.to_string();
        account::Entity::delete_by_id((key, account_id.to_string()))
            .exec(&self.db)
            .await?;
        Ok(())
    }

    async fn list_accounts(&self, provider: &ProviderId) -> Result<Vec<AccountInfo>> {
        let key = provider.to_string();
        let rows = account::Entity::find()
            .filter(account::Column::Provider.eq(&key))
            .order_by_desc(account::Column::IsActive)
            .order_by_asc(account::Column::AccountId)
            .all(&self.db)
            .await?;

        Ok(rows
            .into_iter()
            .map(|m| AccountInfo {
                account_id: m.account_id,
                label: m.label,
                is_active: m.is_active,
            })
            .collect())
    }

    async fn set_active(&self, provider: &ProviderId, account_id: &str) -> Result<()> {
        let key = provider.to_string();

        // Verify the target account exists.
        let target = account::Entity::find_by_id((key.clone(), account_id.to_string()))
            .one(&self.db)
            .await?;

        if target.is_none() {
            return Err(ByokError::Storage(format!(
                "account '{account_id}' not found for provider {provider}"
            )));
        }

        // Deactivate all accounts for this provider.
        db_exec_raw(
            &self.db,
            "UPDATE accounts SET is_active = 0 WHERE provider = ?",
            vec![key.clone().into()],
        )
        .await?;

        // Activate the target.
        db_exec_raw(
            &self.db,
            "UPDATE accounts SET is_active = 1 WHERE provider = ? AND account_id = ?",
            vec![key.into(), account_id.to_string().into()],
        )
        .await?;

        Ok(())
    }

    async fn load_all_tokens(&self, provider: &ProviderId) -> Result<Vec<(String, OAuthToken)>> {
        let key = provider.to_string();
        let rows = account::Entity::find()
            .filter(account::Column::Provider.eq(&key))
            .order_by_desc(account::Column::IsActive)
            .order_by_asc(account::Column::AccountId)
            .all(&self.db)
            .await?;

        let mut result = Vec::with_capacity(rows.len());
        for m in rows {
            let token: OAuthToken = serde_json::from_str(&m.token_json)
                .map_err(|e| ByokError::Storage(e.to_string()))?;
            result.push((m.account_id, token));
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{ConnectionTrait, Database, Statement};

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
        let db = Database::connect("sqlite::memory:").await.unwrap();
        db.execute(Statement::from_string(
            db.get_database_backend(),
            "CREATE TABLE tokens (
                provider   TEXT PRIMARY KEY,
                token_json TEXT NOT NULL
            )"
            .to_string(),
        ))
        .await
        .unwrap();
        let tok = OAuthToken::new("legacy-token");
        let json = serde_json::to_string(&tok).unwrap();
        db.execute(Statement::from_sql_and_values(
            db.get_database_backend(),
            "INSERT INTO tokens (provider, token_json) VALUES ('claude', ?)",
            vec![json.into()],
        ))
        .await
        .unwrap();

        // Run migration.
        SqliteTokenStore::migrate(&db).await.unwrap();

        // Legacy table should be gone.
        let result = db
            .query_one(Statement::from_string(
                db.get_database_backend(),
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='tokens')"
                    .to_string(),
            ))
            .await
            .unwrap();
        let legacy_exists = result
            .and_then(|r| r.try_get_by_index::<bool>(0).ok())
            .unwrap_or(true);
        assert!(!legacy_exists);

        // Data should be in the new table as active "default" account.
        let row = account::Entity::find_by_id(("claude".to_string(), "default".to_string()))
            .one(&db)
            .await
            .unwrap();
        let row = row.unwrap();
        assert_eq!(row.account_id, "default");
        assert!(row.is_active);
    }
}

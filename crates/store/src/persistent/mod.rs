//! SQLite-backed token store using `SeaORM`.
//!
//! Schema: `accounts(provider, account_id, label, is_active, token_json, created_at, updated_at)`
//! with composite primary key `(provider, account_id)`.
//!
//! A partial unique index ensures at most one active account per provider.
//!
//! ## Sub-modules
//!
//! - [`token`] — [`TokenStore`] implementation.
//! - [`history`] — [`ChatHistoryStore`] implementation.

mod history;
mod token;

use sea_orm::{ConnectionTrait, Database, DatabaseConnection, Statement};
use std::time::{SystemTime, UNIX_EPOCH};

/// A persistent [`TokenStore`](byokey_types::TokenStore) backed by `SQLite` via `SeaORM`.
pub struct SqliteTokenStore {
    /// `SeaORM` database connection.
    db: DatabaseConnection,
}

pub(crate) fn now_unix() -> i64 {
    #[allow(clippy::cast_possible_wrap)]
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    secs
}

impl SqliteTokenStore {
    /// Connects to a `SQLite` database (e.g. `"sqlite:./tokens.db?mode=rwc"` or `"sqlite::memory:"`).
    ///
    /// Automatically creates the database file if it does not exist.
    /// Runs migrations to create / upgrade the schema.
    ///
    /// # Errors
    ///
    /// Returns a [`sea_orm::DbErr`] if the connection or table creation fails.
    pub async fn new(database_url: &str) -> std::result::Result<Self, sea_orm::DbErr> {
        let db = Database::connect(database_url).await?;
        Self::migrate(&db).await?;
        Ok(Self { db })
    }

    /// Exposes the inner `DatabaseConnection` for reuse (e.g. future tables).
    #[must_use]
    pub fn connection(&self) -> &DatabaseConnection {
        &self.db
    }

    /// Run schema migrations.
    ///
    /// - Creates the `accounts` table if it does not exist.
    /// - Creates the partial unique index on `(provider)` where `is_active = 1`.
    /// - Migrates from the legacy `tokens` table if present.
    async fn migrate(db: &DatabaseConnection) -> std::result::Result<(), sea_orm::DbErr> {
        // Create the accounts table (idempotent).
        db.execute(Statement::from_string(
            db.get_database_backend(),
            "CREATE TABLE IF NOT EXISTS accounts (
                provider    TEXT    NOT NULL,
                account_id  TEXT    NOT NULL,
                label       TEXT,
                is_active   INTEGER NOT NULL DEFAULT 1,
                token_json  TEXT    NOT NULL,
                created_at  INTEGER NOT NULL DEFAULT (unixepoch()),
                updated_at  INTEGER NOT NULL DEFAULT (unixepoch()),
                PRIMARY KEY (provider, account_id)
            )"
            .to_string(),
        ))
        .await?;

        // Partial unique index: at most one active account per provider.
        db.execute(Statement::from_string(
            db.get_database_backend(),
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_active_account
             ON accounts(provider) WHERE is_active = 1"
                .to_string(),
        ))
        .await?;

        // Migrate legacy `tokens` table if it exists.
        let legacy_rows = db
            .query_one(Statement::from_string(
                db.get_database_backend(),
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='tokens')"
                    .to_string(),
            ))
            .await?;

        let legacy_exists = legacy_rows
            .and_then(|r| r.try_get_by_index::<bool>(0).ok())
            .unwrap_or(false);

        if legacy_exists {
            db.execute(Statement::from_string(
                db.get_database_backend(),
                "INSERT OR IGNORE INTO accounts (provider, account_id, is_active, token_json)
                 SELECT provider, 'default', 1, token_json FROM tokens"
                    .to_string(),
            ))
            .await?;
            db.execute(Statement::from_string(
                db.get_database_backend(),
                "DROP TABLE tokens".to_string(),
            ))
            .await?;
        }

        // ── Conversation history tables ──────────────────────────────────────

        db.execute(Statement::from_string(
            db.get_database_backend(),
            "CREATE TABLE IF NOT EXISTS conversations (
                id          TEXT    PRIMARY KEY,
                title       TEXT,
                model       TEXT    NOT NULL,
                provider    TEXT    NOT NULL,
                created_at  INTEGER NOT NULL DEFAULT (unixepoch()),
                updated_at  INTEGER NOT NULL DEFAULT (unixepoch())
            )"
            .to_string(),
        ))
        .await?;

        db.execute(Statement::from_string(
            db.get_database_backend(),
            "CREATE TABLE IF NOT EXISTS messages (
                id              TEXT    PRIMARY KEY,
                conversation_id TEXT    NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
                role            TEXT    NOT NULL,
                content         TEXT    NOT NULL DEFAULT '',
                input_tokens    INTEGER,
                output_tokens   INTEGER,
                model           TEXT,
                finish_reason   TEXT,
                duration_ms     INTEGER,
                extra_json      TEXT,
                created_at      INTEGER NOT NULL DEFAULT (unixepoch())
            )"
            .to_string(),
        ))
        .await?;

        db.execute(Statement::from_string(
            db.get_database_backend(),
            "CREATE INDEX IF NOT EXISTS idx_messages_conversation
             ON messages(conversation_id, created_at)"
                .to_string(),
        ))
        .await?;

        Ok(())
    }
}

/// Helper to execute a raw SQL statement with positional parameters.
pub(crate) async fn db_exec_raw(
    db: &DatabaseConnection,
    sql: &str,
    values: Vec<sea_orm::Value>,
) -> std::result::Result<(), sea_orm::DbErr> {
    let stmt = Statement::from_sql_and_values(db.get_database_backend(), sql, values);
    db.execute(stmt).await?;
    Ok(())
}

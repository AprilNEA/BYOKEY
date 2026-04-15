//! SQLite-backed token store using `SeaORM`.
//!
//! Schema management is handled by [`crate::migration::Migrator`].
//!
//! ## Sub-modules
//!
//! - [`token`] — [`TokenStore`] implementation.
//! - [`history`] — [`ChatHistoryStore`] implementation.

mod history;
mod token;
mod usage;

use byokey_types::OAuthToken;
use sea_orm::{ConnectOptions, ConnectionTrait, Database, DatabaseConnection, Statement};
use sea_orm_migration::MigratorTrait;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::migration::{self, Migrator};

/// A persistent [`TokenStore`](byokey_types::TokenStore) backed by `SQLite` via `SeaORM`.
pub struct SqliteTokenStore {
    /// `SeaORM` database connection.
    db: DatabaseConnection,
    /// In-memory cache of active tokens keyed by provider string.
    cache: Mutex<HashMap<String, OAuthToken>>,
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
    /// Returns a [`sea_orm::DbErr`] if the connection or migrations fail.
    pub async fn new(database_url: &str) -> std::result::Result<Self, sea_orm::DbErr> {
        // sqlx emits every PRAGMA / migration query at INFO; silence it.
        // `max_connections` defaults to 1 for SQLite under sea-orm v2, which
        // can deadlock if a connection isn't released between the migration
        // and the next query. Bump it explicitly.
        let mut opt = ConnectOptions::new(database_url);
        opt.sqlx_logging(false).max_connections(8);
        let db = Database::connect(opt).await?;
        migration::backfill_pre_migration_install(&db).await?;
        Migrator::up(&db, None).await?;
        Ok(Self {
            db,
            cache: Mutex::new(HashMap::new()),
        })
    }

    /// Exposes the inner `DatabaseConnection` for reuse (e.g. future tables).
    #[must_use]
    pub fn connection(&self) -> &DatabaseConnection {
        &self.db
    }
}

/// Helper to execute a raw SQL statement with positional parameters.
pub(crate) async fn db_exec_raw(
    db: &impl ConnectionTrait,
    sql: &str,
    values: Vec<sea_orm::Value>,
) -> std::result::Result<(), sea_orm::DbErr> {
    let stmt = Statement::from_sql_and_values(db.get_database_backend(), sql, values);
    db.execute_raw(stmt).await?;
    Ok(())
}

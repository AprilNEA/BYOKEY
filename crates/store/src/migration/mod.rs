//! Database migrations managed via `sea-orm-migration`.
//!
//! Migrations run automatically at [`crate::SqliteTokenStore::new`] startup.
//! For databases predating this migration system (i.e. those whose schema was
//! created by the previous hand-written `migrate()` function), a one-shot
//! backfill marks the historical migrations as already applied so they don't
//! attempt to recreate existing tables. See [`backfill_pre_migration_install`].

// Wildcard imports of `sea_orm_migration::{prelude::*, schema::*}` are the
// canonical pattern in migration files — the prelude pulls in `Table`,
// `Index`, `Expr`, `OnConflict`, etc. that every migration touches.
#![allow(clippy::wildcard_imports)]

use sea_orm::{ActiveValue::Set, DatabaseConnection, EntityTrait};
use sea_orm_migration::{prelude::*, seaql_migrations};

mod m20260415_000001_create_accounts;
mod m20260415_000002_migrate_legacy_tokens;
mod m20260415_000003_create_conversations;
mod m20260415_000004_create_messages;
mod m20260415_000005_create_usage_records;
mod m20260417_000006_add_usage_account_id;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20260415_000001_create_accounts::Migration),
            Box::new(m20260415_000002_migrate_legacy_tokens::Migration),
            Box::new(m20260415_000003_create_conversations::Migration),
            Box::new(m20260415_000004_create_messages::Migration),
            Box::new(m20260415_000005_create_usage_records::Migration),
            Box::new(m20260417_000006_add_usage_account_id::Migration),
        ]
    }
}

/// Names of migrations that correspond to schema created by the legacy
/// hand-written `migrate()` function (everything up to and including
/// `usage_records`).  Used by [`backfill_pre_migration_install`].
const HISTORICAL_MIGRATIONS: &[&str] = &[
    "m20260415_000001_create_accounts",
    "m20260415_000002_migrate_legacy_tokens",
    "m20260415_000003_create_conversations",
    "m20260415_000004_create_messages",
    "m20260415_000005_create_usage_records",
];

/// Mark the historical migrations as already applied if the database was
/// created by the pre-migration `migrate()` code path.
///
/// Detection: the `accounts` table exists but `seaql_migrations` does not.
/// In that case we install the tracking table and stamp every migration
/// up to and including `usage_records` as applied so [`Migrator::up`] skips
/// them. Any newer migrations added later still run normally.
///
/// # Errors
///
/// Returns [`DbErr`] if the existence probes, tracking-table install, or
/// backfill inserts fail.
pub async fn backfill_pre_migration_install(db: &DatabaseConnection) -> Result<(), DbErr> {
    let manager = SchemaManager::new(db);
    if !manager.has_table("accounts").await? {
        return Ok(());
    }
    if manager.has_table("seaql_migrations").await? {
        return Ok(());
    }

    Migrator::install(db).await?;

    let now = now_unix();
    let rows = HISTORICAL_MIGRATIONS
        .iter()
        .map(|name| seaql_migrations::ActiveModel {
            version: Set((*name).to_string()),
            applied_at: Set(now),
        });
    seaql_migrations::Entity::insert_many(rows)
        .on_conflict_do_nothing_on([seaql_migrations::Column::Version])
        .exec(db)
        .await?;
    Ok(())
}

fn now_unix() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    #[allow(clippy::cast_possible_wrap)]
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    secs
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::Database;

    #[tokio::test]
    async fn fresh_install_runs_all_migrations() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        // No backfill needed on a fresh DB.
        backfill_pre_migration_install(&db).await.unwrap();
        Migrator::up(&db, None).await.unwrap();

        let applied = Migrator::get_applied_migrations(&db).await.unwrap();
        assert_eq!(applied.len(), HISTORICAL_MIGRATIONS.len());
        for (i, m) in applied.iter().enumerate() {
            assert_eq!(m.name(), HISTORICAL_MIGRATIONS[i]);
        }
    }

    #[tokio::test]
    async fn backfill_marks_historical_migrations_as_applied() {
        let db = Database::connect("sqlite::memory:").await.unwrap();

        // Simulate a pre-migration install: create just the `accounts` table
        // by hand, leave `seaql_migrations` absent.
        db.execute_unprepared(
            "CREATE TABLE accounts (
                provider    TEXT NOT NULL,
                account_id  TEXT NOT NULL,
                token_json  TEXT NOT NULL,
                PRIMARY KEY (provider, account_id)
            )",
        )
        .await
        .unwrap();

        backfill_pre_migration_install(&db).await.unwrap();

        // All historical migrations should be marked as applied.
        let applied = Migrator::get_applied_migrations(&db).await.unwrap();
        assert_eq!(applied.len(), HISTORICAL_MIGRATIONS.len());

        // Migrator::up should now be a no-op (no pending migrations).
        let pending = Migrator::get_pending_migrations(&db).await.unwrap();
        assert!(pending.is_empty());
        Migrator::up(&db, None).await.unwrap();
    }

    #[tokio::test]
    async fn backfill_is_idempotent() {
        // Second call on a fully-migrated DB should be a no-op (the
        // `seaql_migrations` table already exists).
        let db = Database::connect("sqlite::memory:").await.unwrap();
        Migrator::up(&db, None).await.unwrap();

        let before = Migrator::get_applied_migrations(&db).await.unwrap().len();
        backfill_pre_migration_install(&db).await.unwrap();
        let after = Migrator::get_applied_migrations(&db).await.unwrap().len();
        assert_eq!(before, after);
    }
}

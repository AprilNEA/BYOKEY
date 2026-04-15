use sea_orm_migration::prelude::*;

use super::m20260415_000001_create_accounts::Accounts;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[derive(DeriveIden)]
enum Tokens {
    Table,
    Provider,
    TokenJson,
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        if !manager.has_table(Tokens::Table.to_string()).await? {
            return Ok(());
        }

        let select = Query::select()
            .column(Tokens::Provider)
            .expr(Expr::val("default"))
            .expr(Expr::val(true))
            .column(Tokens::TokenJson)
            .from(Tokens::Table)
            .to_owned();

        // The migration runs exactly once (tracked by `seaql_migrations`) and
        // `tokens.provider` is itself unique, so no `ON CONFLICT` clause is
        // needed. (SQLite's parser also rejects `ON CONFLICT` directly after a
        // `SELECT` with no `WHERE`, mistaking it for a JOIN's `ON`.)
        let mut insert = Query::insert();
        insert
            .into_table(Accounts::Table)
            .columns([
                Accounts::Provider,
                Accounts::AccountId,
                Accounts::IsActive,
                Accounts::TokenJson,
            ])
            .select_from(select)
            .map_err(|e| DbErr::Migration(e.to_string()))?;
        manager.exec_stmt(insert).await?;

        manager
            .drop_table(Table::drop().table(Tokens::Table).to_owned())
            .await?;
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        // Cannot recreate the legacy `tokens` table from `accounts` because the
        // original column set and timestamps are lost.
        Err(DbErr::Migration(
            "legacy tokens migration is not reversible".into(),
        ))
    }
}

use sea_orm_migration::{prelude::*, schema::*};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[derive(DeriveIden)]
pub enum Accounts {
    Table,
    Provider,
    AccountId,
    Label,
    IsActive,
    TokenJson,
    CreatedAt,
    UpdatedAt,
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Accounts::Table)
                    .if_not_exists()
                    .col(text(Accounts::Provider))
                    .col(text(Accounts::AccountId))
                    .col(text_null(Accounts::Label))
                    .col(boolean(Accounts::IsActive).default(true))
                    .col(text(Accounts::TokenJson))
                    .col(big_integer(Accounts::CreatedAt).default(Expr::cust("(unixepoch())")))
                    .col(big_integer(Accounts::UpdatedAt).default(Expr::cust("(unixepoch())")))
                    .primary_key(
                        Index::create()
                            .col(Accounts::Provider)
                            .col(Accounts::AccountId),
                    )
                    .to_owned(),
            )
            .await?;

        // Partial unique index — sea-query has no `WHERE` builder for indexes,
        // so the predicate goes through raw SQL.
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE UNIQUE INDEX IF NOT EXISTS idx_active_account
                 ON accounts(provider) WHERE is_active = 1",
            )
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_index(
                Index::drop()
                    .name("idx_active_account")
                    .table(Accounts::Table)
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(Table::drop().table(Accounts::Table).to_owned())
            .await?;
        Ok(())
    }
}

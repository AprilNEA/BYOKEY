//! Add `account_id` column to `usage_records` so usage can be attributed to
//! a specific OAuth account (or `'default'` for API-key / unknown flows).

use sea_orm_migration::{prelude::*, schema::*};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[derive(DeriveIden)]
enum UsageRecords {
    Table,
    AccountId,
    Provider,
    CreatedAt,
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(UsageRecords::Table)
                    .add_column(text(UsageRecords::AccountId).default("default"))
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .if_not_exists()
                    .name("idx_usage_provider_account_created")
                    .table(UsageRecords::Table)
                    .col(UsageRecords::Provider)
                    .col(UsageRecords::AccountId)
                    .col(UsageRecords::CreatedAt)
                    .to_owned(),
            )
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_index(
                Index::drop()
                    .name("idx_usage_provider_account_created")
                    .table(UsageRecords::Table)
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(UsageRecords::Table)
                    .drop_column(UsageRecords::AccountId)
                    .to_owned(),
            )
            .await?;
        Ok(())
    }
}

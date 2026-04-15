use sea_orm_migration::{prelude::*, schema::*};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[derive(DeriveIden)]
pub enum UsageRecords {
    Table,
    Id,
    Model,
    Provider,
    InputTokens,
    OutputTokens,
    Success,
    CreatedAt,
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(UsageRecords::Table)
                    .if_not_exists()
                    .col(pk_auto(UsageRecords::Id))
                    .col(text(UsageRecords::Model))
                    .col(text(UsageRecords::Provider))
                    .col(big_integer(UsageRecords::InputTokens).default(0))
                    .col(big_integer(UsageRecords::OutputTokens).default(0))
                    .col(boolean(UsageRecords::Success).default(true))
                    .col(big_integer(UsageRecords::CreatedAt).default(Expr::cust("(unixepoch())")))
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .if_not_exists()
                    .name("idx_usage_created")
                    .table(UsageRecords::Table)
                    .col(UsageRecords::CreatedAt)
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .if_not_exists()
                    .name("idx_usage_model_created")
                    .table(UsageRecords::Table)
                    .col(UsageRecords::Model)
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
                    .name("idx_usage_model_created")
                    .table(UsageRecords::Table)
                    .to_owned(),
            )
            .await?;
        manager
            .drop_index(
                Index::drop()
                    .name("idx_usage_created")
                    .table(UsageRecords::Table)
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(Table::drop().table(UsageRecords::Table).to_owned())
            .await?;
        Ok(())
    }
}

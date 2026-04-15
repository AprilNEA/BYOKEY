use sea_orm_migration::{prelude::*, schema::*};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[derive(DeriveIden)]
pub enum Conversations {
    Table,
    Id,
    Title,
    Model,
    Provider,
    CreatedAt,
    UpdatedAt,
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Conversations::Table)
                    .if_not_exists()
                    .col(text(Conversations::Id).primary_key())
                    .col(text_null(Conversations::Title))
                    .col(text(Conversations::Model))
                    .col(text(Conversations::Provider))
                    .col(big_integer(Conversations::CreatedAt).default(Expr::cust("(unixepoch())")))
                    .col(big_integer(Conversations::UpdatedAt).default(Expr::cust("(unixepoch())")))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Conversations::Table).to_owned())
            .await
    }
}

use sea_orm_migration::{prelude::*, schema::*};

use super::m20260415_000003_create_conversations::Conversations;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[derive(DeriveIden)]
pub enum Messages {
    Table,
    Id,
    ConversationId,
    Role,
    Content,
    InputTokens,
    OutputTokens,
    Model,
    FinishReason,
    DurationMs,
    ExtraJson,
    CreatedAt,
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Messages::Table)
                    .if_not_exists()
                    .col(text(Messages::Id).primary_key())
                    .col(text(Messages::ConversationId))
                    .col(text(Messages::Role))
                    .col(text(Messages::Content).default(""))
                    .col(big_integer_null(Messages::InputTokens))
                    .col(big_integer_null(Messages::OutputTokens))
                    .col(text_null(Messages::Model))
                    .col(text_null(Messages::FinishReason))
                    .col(big_integer_null(Messages::DurationMs))
                    .col(text_null(Messages::ExtraJson))
                    .col(big_integer(Messages::CreatedAt).default(Expr::cust("(unixepoch())")))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_messages_conversation")
                            .from(Messages::Table, Messages::ConversationId)
                            .to(Conversations::Table, Conversations::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .if_not_exists()
                    .name("idx_messages_conversation")
                    .table(Messages::Table)
                    .col(Messages::ConversationId)
                    .col(Messages::CreatedAt)
                    .to_owned(),
            )
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_index(
                Index::drop()
                    .name("idx_messages_conversation")
                    .table(Messages::Table)
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(Table::drop().table(Messages::Table).to_owned())
            .await?;
        Ok(())
    }
}

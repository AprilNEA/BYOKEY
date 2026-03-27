//! `SeaORM` entity for the `accounts` table.

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "accounts")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub provider: String,
    #[sea_orm(primary_key, auto_increment = false)]
    pub account_id: String,
    pub label: Option<String>,
    pub is_active: bool,
    #[sea_orm(column_type = "Text")]
    pub token_json: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

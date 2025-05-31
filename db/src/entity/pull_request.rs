use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "pull_requests")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub title: String,
    pub body: Option<String>,
    pub state: PullRequestState,
    pub author: String,
    pub updated_at: DateTimeUtc,
}

#[derive(Debug, Clone, PartialEq, EnumIter, DeriveActiveEnum)]
#[sea_orm(rs_type = "String", db_type = "String(Some(16))")]
pub enum PullRequestState {
    #[sea_orm(string_value = "Open")]
    Open,
    #[sea_orm(string_value = "Closed")]
    Closed,
    #[sea_orm(string_value = "Merged")]
    Merged,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

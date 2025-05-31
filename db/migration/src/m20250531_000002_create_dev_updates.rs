use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

// You might want to change the filename to something like:
// m20250531_000002_create_dev_updates.rs

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    // Apply migration: create dev_updates table
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.create_table(
            Table::create()
                .table(DevUpdates::Table)
                .if_not_exists()
                .col(
                    ColumnDef::new(DevUpdates::Id)
                        .integer()
                        .not_null()
                        .auto_increment()
                        .primary_key(),
                )
                .col(ColumnDef::new(DevUpdates::StartDate).date().not_null())
                .col(ColumnDef::new(DevUpdates::EndDate).date().not_null())
                .to_owned(),
        ).await
    }

    // Rollback migration: drop dev_updates table
    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(
            Table::drop()
                .table(DevUpdates::Table)
                .to_owned(),
        ).await
    }
}

#[derive(Iden)]
enum DevUpdates {
    Table,
    Id,
    StartDate,
    EndDate,
}

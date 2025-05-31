use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    // Forward migration: create the pull_requests table with a status enum column
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(PullRequests::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(PullRequests::Id)
                            .integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(PullRequests::Title).string().not_null())
                    .col(ColumnDef::new(PullRequests::Body).text().null())
                    .col(ColumnDef::new(PullRequests::Author).string().not_null())
                    .col(ColumnDef::new(PullRequests::UpdatedAt).date_time().not_null())
                    // Add the state column as string
                    .col(
                        ColumnDef::new(PullRequests::State)
                            .string()
                            .not_null()
                            // Optionally set a default state:
                            .default("Open")
                    )
                    .to_owned(),
            )
            .await
    }

    // Rollback migration: drop the pull_requests table
    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(PullRequests::Table).to_owned())
            .await
    }
}

#[derive(Iden)]
enum PullRequests {
    Table,
    Id,
    Title,
    Body,
    State,
    Author,
    UpdatedAt,
}

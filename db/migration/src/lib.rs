use sea_orm_migration::prelude::*;

mod m20250531_000001_create_pull_requests;
mod m20250531_000002_create_dev_updates;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20250531_000001_create_pull_requests::Migration),
            Box::new(m20250531_000002_create_dev_updates::Migration),
        ]
    }
}

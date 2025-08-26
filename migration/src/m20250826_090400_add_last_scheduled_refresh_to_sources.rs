use loco_rs::schema::*;
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.alter_table(
            Table::alter()
                .table(Sources::Table)
                .add_column(timestamp_null(Sources::LastScheduledRefresh))
                .to_owned(),
        )
        .await
    }

    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.alter_table(
            Table::alter()
                .table(Sources::Table)
                .drop_column(Sources::LastScheduledRefresh)
                .to_owned(),
        )
        .await
    }
}

#[derive(DeriveIden)]
enum Sources {
    Table,
    LastScheduledRefresh,
}

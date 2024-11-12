use loco_rs::schema::table_auto_tz;
use sea_orm_migration::{prelude::*, schema::*};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                table_auto_tz(Sources::Table)
                    .col(pk_auto(Sources::Id))
                    .col(string(Sources::Url))
                    .col(integer(Sources::FetchLastDays))
                    .col(timestamp_null(Sources::LastRefreshedAt))
                    .col(integer(Sources::RefreshFrequency))
                    .col(string(Sources::Sponsorblock))
                    .col(json_null(Sources::Metadata))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Sources::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum Sources {
    Table,
    Id,
    Url,
    FetchLastDays,
    LastRefreshedAt,
    RefreshFrequency,
    Sponsorblock,
    Metadata,
}

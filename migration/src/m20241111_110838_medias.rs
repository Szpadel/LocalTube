use loco_rs::schema::table_auto_tz;
use sea_orm_migration::{prelude::*, schema::*};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                table_auto_tz(Medias::Table)
                    .col(pk_auto(Medias::Id))
                    .col(string(Medias::Url))
                    .col(integer(Medias::SourceId))
                    .col(json_null(Medias::Metadata))
                    .col(string_null(Medias::MediaPath))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk-medias-sources")
                            .from(Medias::Table, Medias::SourceId)
                            .to(Sources::Table, Sources::Id)
                            .on_delete(ForeignKeyAction::Cascade)
                            .on_update(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Medias::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum Medias {
    Table,
    Id,
    Url,
    SourceId,
    Metadata,
    MediaPath,
}

#[derive(DeriveIden)]
enum Sources {
    Table,
    Id,
}

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(
                Table::drop()
                    .table(JobTables::SqltLocoQueue)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .table(JobTables::SqltLocoQueueLock)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .table(JobTables::PgLocoQueue)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}

#[derive(DeriveIden)]
enum JobTables {
    #[sea_orm(iden = "sqlt_loco_queue")]
    SqltLocoQueue,
    #[sea_orm(iden = "sqlt_loco_queue_lock")]
    SqltLocoQueueLock,
    #[sea_orm(iden = "pg_loco_queue")]
    PgLocoQueue,
}

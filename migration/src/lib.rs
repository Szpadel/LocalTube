#![allow(elided_lifetimes_in_paths)]
#![allow(clippy::wildcard_imports)]
pub use sea_orm_migration::prelude::*;

mod m20220101_000001_users;

mod m20241110_170457_sources;
mod m20241111_110838_medias;
mod m20250826_090400_add_last_scheduled_refresh_to_sources;
mod m20260105_000000_drop_loco_job_tables;
pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            // inject-below (do not remove this comment)
            Box::new(m20260105_000000_drop_loco_job_tables::Migration),
            Box::new(m20241111_110838_medias::Migration),
            Box::new(m20241110_170457_sources::Migration),
            Box::new(m20220101_000001_users::Migration),
            Box::new(m20250826_090400_add_last_scheduled_refresh_to_sources::Migration),
            // inject-above (do not remove this comment)
        ]
    }
}

//! `SeaORM` Entity, @generated by sea-orm-codegen 1.1.1

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq, Serialize, Deserialize)]
#[sea_orm(table_name = "sources")]
pub struct Model {
    pub created_at: DateTimeWithTimeZone,
    pub updated_at: DateTimeWithTimeZone,
    #[sea_orm(primary_key)]
    pub id: i32,
    pub url: String,
    pub fetch_last_days: i32,
    pub last_refreshed_at: Option<DateTimeUtc>,
    pub refresh_frequency: i32,
    pub sponsorblock: String,
    pub metadata: Option<Json>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::medias::Entity")]
    Medias,
}

impl Related<super::medias::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Medias.def()
    }
}

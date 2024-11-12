#![allow(clippy::missing_errors_doc)]
#![allow(clippy::unnecessary_struct_initialization)]
#![allow(clippy::unused_async)]
use axum::debug_handler;
use loco_rs::prelude::*;
use sea_orm::{sea_query::Order, QueryOrder, EntityTrait};

use crate::{
    models::_entities::medias::{Column, Entity, Model},
    views,
};

async fn load_item(
    ctx: &AppContext,
    id: i32,
) -> Result<(Model, Option<crate::models::_entities::sources::Model>)> {
    let items = Entity::find_by_id(id)
        .find_with_related(crate::models::_entities::sources::Entity)
        .all(&ctx.db)
        .await?;
    items.into_iter()
        .next()
        .map(|(media, sources)| (media, sources.into_iter().next()))
        .ok_or_else(|| Error::NotFound)
}

#[debug_handler]
pub async fn list(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
) -> Result<Response> {
    let items = Entity::find()
        .find_also_related(crate::models::_entities::sources::Entity)
        .order_by(Column::Id, Order::Desc)
        .all(&ctx.db)
        .await?;
    views::media::list(&v, &items)
}

#[debug_handler]
pub async fn show(
    Path(id): Path<i32>,
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
) -> Result<Response> {
    let (item, source) = load_item(&ctx, id).await?;
    views::media::show(&v, &item, source.as_ref())
}

pub fn routes() -> Routes {
    Routes::new()
        .prefix("medias/")
        .add("/", get(list))
        .add(":id", get(show))
}

#![allow(clippy::missing_errors_doc)]
#![allow(clippy::unnecessary_struct_initialization)]
#![allow(clippy::unused_async)]
use axum::{debug_handler, response::Redirect};
use loco_rs::prelude::*;
use sea_orm::{sea_query::Order, EntityTrait, QueryOrder, Set};

use crate::{
    models::_entities::medias::{ActiveModel, Column, Entity, Model},
    views,
    workers::fetch_media::{FetchMediaWorker, FetchMediaWorkerArgs},
};

async fn load_item(
    ctx: &AppContext,
    id: i32,
) -> Result<(Model, Option<crate::models::_entities::sources::Model>)> {
    let items = Entity::find_by_id(id)
        .find_with_related(crate::models::_entities::sources::Entity)
        .all(&ctx.db)
        .await?;
    items
        .into_iter()
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

#[debug_handler]
pub async fn redownload(Path(id): Path<i32>, State(ctx): State<AppContext>) -> Result<Redirect> {
    let (item, _) = load_item(&ctx, id).await?;

    // Remove existing media files from filesystem
    item.remove_media_files()?;

    // Update database to clear media_path
    let media_update = ActiveModel {
        id: Set(item.id),
        media_path: Set(None),
        ..Default::default()
    };
    Entity::update(media_update).exec(&ctx.db).await?;

    // Queue new download job
    FetchMediaWorker::perform_later(&ctx, FetchMediaWorkerArgs { media_id: item.id }).await?;

    // Redirect back to media list (303 See Other forces GET method)
    Ok(Redirect::to("/medias"))
}

pub fn routes() -> Routes {
    Routes::new()
        .prefix("medias/")
        .add("/", get(list))
        .add("{id}", get(show))
        .add("{id}/redownload", post(redownload))
}

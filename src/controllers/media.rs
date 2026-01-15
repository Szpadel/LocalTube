#![allow(clippy::missing_errors_doc)]
#![allow(clippy::unnecessary_struct_initialization)]
#![allow(clippy::unused_async)]
use axum::{body::Bytes, debug_handler, http::header, response::Redirect};
use futures_util::stream;
use loco_rs::prelude::*;
use sea_orm::{sea_query::Order, EntityTrait, QueryOrder, Set};
use std::path::Component;
use tokio::io::AsyncReadExt;

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

fn content_type_for(path: &std::path::Path) -> &'static str {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .map(str::to_ascii_lowercase);
    match ext.as_deref() {
        Some("mp4") => "video/mp4",
        Some("webm") => "video/webm",
        Some("mkv") => "video/x-matroska",
        Some("mov") => "video/quicktime",
        Some("avi") => "video/x-msvideo",
        _ => "application/octet-stream",
    }
}

#[debug_handler]
pub async fn stream(Path(id): Path<i32>, State(ctx): State<AppContext>) -> Result<Response> {
    let (item, _) = load_item(&ctx, id).await?;
    let Some(media_path) = item.media_path else {
        return Err(Error::NotFound);
    };

    let rel_path = std::path::PathBuf::from(media_path);
    if rel_path.is_absolute()
        || rel_path
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::Prefix(_)))
    {
        return Err(Error::NotFound);
    }

    let full_path = crate::ytdlp::media_directory().join(&rel_path);
    let metadata = tokio::fs::metadata(&full_path)
        .await
        .map_err(|_| Error::NotFound)?;
    let file = tokio::fs::File::open(&full_path)
        .await
        .map_err(|_| Error::NotFound)?;

    let stream = stream::unfold(file, |mut file| async {
        let mut buffer = vec![0u8; 16 * 1024];
        match file.read(&mut buffer).await {
            Ok(0) => None,
            Ok(read) => Some((
                Ok::<Bytes, std::io::Error>(Bytes::copy_from_slice(&buffer[..read])),
                file,
            )),
            Err(err) => Some((Err(err), file)),
        }
    });

    let mut response = Response::new(axum::body::Body::from_stream(stream));
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static(content_type_for(&full_path)),
    );
    if let Ok(value) = header::HeaderValue::from_str(&metadata.len().to_string()) {
        headers.insert(header::CONTENT_LENGTH, value);
    }

    Ok(response)
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
        .add("{id}/stream", get(stream))
        .add("{id}/redownload", post(redownload))
}

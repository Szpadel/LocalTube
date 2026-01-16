#![allow(clippy::missing_errors_doc)]
#![allow(clippy::unnecessary_struct_initialization)]
#![allow(clippy::unused_async)]
use axum::{
    body::Bytes,
    debug_handler,
    http::{header, HeaderMap, StatusCode},
    response::Redirect,
};
use futures_util::stream;
use loco_rs::prelude::*;
use sea_orm::{sea_query::Order, EntityTrait, QueryOrder, Set};
use std::path::Component;
use tokio::io::{AsyncReadExt, AsyncSeekExt, SeekFrom};

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

fn is_multi_range_header(range: &str) -> bool {
    let value = range.trim();
    value.starts_with("bytes=") && value.trim_start_matches("bytes=").contains(',')
}

fn parse_range_header(range: &str, file_size: u64) -> Option<(u64, u64)> {
    if file_size == 0 {
        return None;
    }

    let value = range.trim();
    if !value.starts_with("bytes=") {
        return None;
    }

    let range_value = value.trim_start_matches("bytes=");
    let mut parts = range_value.split(',').next()?.trim().splitn(2, '-');
    let start_part = parts.next().unwrap_or("");
    let end_part = parts.next().unwrap_or("");

    if start_part.is_empty() {
        let suffix_len: u64 = end_part.parse().ok()?;
        if suffix_len == 0 {
            return None;
        }
        if suffix_len >= file_size {
            return Some((0, file_size - 1));
        }
        let start = file_size - suffix_len;
        let end = file_size - 1;
        return Some((start, end));
    }

    let start: u64 = start_part.parse().ok()?;
    if start >= file_size {
        return None;
    }

    let end = if end_part.is_empty() {
        file_size - 1
    } else {
        let end: u64 = end_part.parse().ok()?;
        if end < start {
            return None;
        }
        std::cmp::min(end, file_size - 1)
    };

    Some((start, end))
}

fn range_not_satisfiable_response(file_size: u64) -> Response {
    let mut response = Response::new(axum::body::Body::empty());
    *response.status_mut() = StatusCode::RANGE_NOT_SATISFIABLE;
    let response_headers = response.headers_mut();
    response_headers.insert(
        header::ACCEPT_RANGES,
        header::HeaderValue::from_static("bytes"),
    );
    if let Ok(value) = header::HeaderValue::from_str(&format!("bytes */{file_size}")) {
        response_headers.insert(header::CONTENT_RANGE, value);
    }
    response
}

fn stream_body(file: tokio::fs::File, remaining: u64) -> axum::body::Body {
    let stream = stream::unfold((file, remaining), |(mut file, mut remaining)| async move {
        if remaining == 0 {
            return None;
        }
        let chunk_size = match usize::try_from(remaining) {
            Ok(remaining) => std::cmp::min(16 * 1024, remaining),
            Err(_) => 16 * 1024,
        };
        let mut buffer = vec![0u8; chunk_size];
        match file.read(&mut buffer).await {
            Ok(0) => None,
            Ok(read) => {
                remaining = remaining.saturating_sub(read as u64);
                buffer.truncate(read);
                Some((
                    Ok::<Bytes, std::io::Error>(Bytes::copy_from_slice(&buffer)),
                    (file, remaining),
                ))
            }
            Err(err) => Some((Err(err), (file, remaining))),
        }
    });

    axum::body::Body::from_stream(stream)
}

#[debug_handler]
pub async fn stream(
    Path(id): Path<i32>,
    State(ctx): State<AppContext>,
    headers: HeaderMap,
) -> Result<Response> {
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
    let mut file = tokio::fs::File::open(&full_path)
        .await
        .map_err(|_| Error::NotFound)?;

    let file_size = metadata.len();
    let range_header = headers
        .get(header::RANGE)
        .and_then(|value| value.to_str().ok());
    let is_bytes_range = range_header.is_some_and(|value| value.trim().starts_with("bytes="));
    let is_multi_range = range_header
        .filter(|_| is_bytes_range)
        .is_some_and(is_multi_range_header);
    let range = if is_multi_range || !is_bytes_range {
        None
    } else {
        range_header.and_then(|value| parse_range_header(value, file_size))
    };

    if is_bytes_range && range.is_none() && !is_multi_range {
        return Ok(range_not_satisfiable_response(file_size));
    }

    let (start, end, status) = if let Some((start, end)) = range {
        (start, end, StatusCode::PARTIAL_CONTENT)
    } else if file_size == 0 {
        (0, 0, StatusCode::OK)
    } else {
        (0, file_size - 1, StatusCode::OK)
    };

    if start > 0 {
        file.seek(SeekFrom::Start(start))
            .await
            .map_err(|_| Error::NotFound)?;
    }

    let remaining = if file_size == 0 {
        0
    } else {
        end.saturating_sub(start).saturating_add(1)
    };

    let mut response = Response::new(stream_body(file, remaining));
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static(content_type_for(&full_path)),
    );
    headers.insert(
        header::ACCEPT_RANGES,
        header::HeaderValue::from_static("bytes"),
    );
    if let Ok(value) = header::HeaderValue::from_str(&remaining.to_string()) {
        headers.insert(header::CONTENT_LENGTH, value);
    }
    if status == StatusCode::PARTIAL_CONTENT {
        if let Ok(value) =
            header::HeaderValue::from_str(&format!("bytes {start}-{end}/{file_size}"))
        {
            headers.insert(header::CONTENT_RANGE, value);
        }
    }
    *response.status_mut() = status;

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

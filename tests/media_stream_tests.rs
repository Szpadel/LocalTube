use axum::http::{header, StatusCode};
use localtube::{app::App, models::_entities};
use loco_rs::prelude::*;
use sea_orm::{ActiveModelTrait, Set};
use serial_test::serial;
use std::path::PathBuf;
use uuid::Uuid;

struct TempMediaFile {
    rel_path: String,
    full_path: PathBuf,
    cleanup_dir: PathBuf,
}

impl TempMediaFile {
    fn new(content: &[u8]) -> Self {
        let media_dir = localtube::ytdlp::media_directory();
        let cleanup_dir = media_dir.join("test_streaming");
        std::fs::create_dir_all(&cleanup_dir).expect("media test directory should be created");

        let filename = format!("stream_{}.mp4", Uuid::new_v4());
        let rel_path = format!("test_streaming/{filename}");
        let full_path = cleanup_dir.join(&filename);
        std::fs::write(&full_path, content).expect("media test file should be created");

        Self {
            rel_path,
            full_path,
            cleanup_dir,
        }
    }
}

impl Drop for TempMediaFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.full_path);
        let _ = std::fs::remove_dir(&self.cleanup_dir);
    }
}

async fn create_media(ctx: &AppContext, rel_path: &str) -> _entities::medias::Model {
    let source = _entities::sources::ActiveModel {
        url: Set("https://example.com/source".to_string()),
        fetch_last_days: Set(7),
        refresh_frequency: Set(24),
        sponsorblock: Set("all".to_string()),
        last_refreshed_at: Set(None),
        last_scheduled_refresh: Set(None),
        metadata: Set(None),
        ..Default::default()
    };
    let source = source
        .insert(&ctx.db)
        .await
        .expect("source should be inserted");

    let media = _entities::medias::ActiveModel {
        url: Set("https://example.com/video".to_string()),
        source_id: Set(source.id),
        metadata: Set(None),
        media_path: Set(Some(rel_path.to_string())),
        ..Default::default()
    };

    media
        .insert(&ctx.db)
        .await
        .expect("media should be inserted")
}

#[tokio::test]
#[serial]
async fn stream_returns_full_body() {
    request_with_create_db::<App, _, _>(|request, ctx| async move {
        let content = b"0123456789";
        let temp = TempMediaFile::new(content);
        let media = create_media(&ctx, &temp.rel_path).await;

        let response = request.get(&format!("/medias/{}/stream", media.id)).await;

        assert_eq!(response.status_code(), StatusCode::OK);
        assert_eq!(
            response
                .header(header::ACCEPT_RANGES)
                .to_str()
                .expect("accept ranges header should be valid"),
            "bytes"
        );
        assert_eq!(
            response
                .header(header::CONTENT_LENGTH)
                .to_str()
                .expect("content length header should be valid"),
            content.len().to_string()
        );
        assert_eq!(response.as_bytes().as_ref(), content);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn stream_honors_single_range_request() {
    request_with_create_db::<App, _, _>(|request, ctx| async move {
        let content = b"0123456789";
        let temp = TempMediaFile::new(content);
        let media = create_media(&ctx, &temp.rel_path).await;

        let response = request
            .get(&format!("/medias/{}/stream", media.id))
            .add_header(header::RANGE, "bytes=2-5")
            .await;

        assert_eq!(response.status_code(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(
            response
                .header(header::CONTENT_RANGE)
                .to_str()
                .expect("content range header should be valid"),
            format!("bytes 2-5/{}", content.len())
        );
        assert_eq!(
            response
                .header(header::CONTENT_LENGTH)
                .to_str()
                .expect("content length header should be valid"),
            "4"
        );
        assert_eq!(response.as_bytes().as_ref(), &content[2..=5]);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn stream_rejects_invalid_range_request() {
    request_with_create_db::<App, _, _>(|request, ctx| async move {
        let content = b"0123456789";
        let temp = TempMediaFile::new(content);
        let media = create_media(&ctx, &temp.rel_path).await;

        let response = request
            .get(&format!("/medias/{}/stream", media.id))
            .add_header(header::RANGE, "bytes=999-1000")
            .await;

        assert_eq!(response.status_code(), StatusCode::RANGE_NOT_SATISFIABLE);
        assert_eq!(
            response
                .header(header::CONTENT_RANGE)
                .to_str()
                .expect("content range header should be valid"),
            format!("bytes */{}", content.len())
        );
        assert!(response.as_bytes().is_empty());
    })
    .await;
}

#[tokio::test]
#[serial]
async fn stream_ignores_unsupported_range_unit() {
    request_with_create_db::<App, _, _>(|request, ctx| async move {
        let content = b"0123456789";
        let temp = TempMediaFile::new(content);
        let media = create_media(&ctx, &temp.rel_path).await;

        let response = request
            .get(&format!("/medias/{}/stream", media.id))
            .add_header(header::RANGE, "items=0-3")
            .await;

        assert_eq!(response.status_code(), StatusCode::OK);
        assert_eq!(response.as_bytes().as_ref(), content);
    })
    .await;
}

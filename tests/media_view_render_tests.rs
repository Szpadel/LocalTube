use axum::body;
use chrono::DateTime;
use localtube::{
    initializers::view_engine::build_test_tera_engine,
    models::{
        _entities::medias, _entities::sources, medias::MediaMetadata, sources::SourceMetadata,
    },
    views,
};
use loco_rs::prelude::*;

/// 2026-09-30 19:08:51 UTC.
const UPLOAD_TIMESTAMP: i64 = 1_790_795_331;
/// 46 minutes and 25 seconds.
const DURATION_SECONDS: u64 = 2785;

fn sample_timestamp() -> DateTime<chrono::FixedOffset> {
    DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z").expect("timestamp should parse")
}

fn sample_source() -> sources::Model {
    let metadata = SourceMetadata {
        uploader: "Test Channel".to_string(),
        items: 2,
        source_provider: "Youtube".to_string(),
        list_kind: None,
        list_count: None,
        list_order: None,
        list_tab: None,
        list_tabs: None,
    };
    sources::Model {
        created_at: sample_timestamp(),
        updated_at: sample_timestamp(),
        id: 1,
        url: "https://example.com/channel".to_string(),
        fetch_last_days: 14,
        last_refreshed_at: None,
        refresh_frequency: 24,
        sponsorblock: String::new(),
        metadata: Some(serde_json::to_value(metadata).expect("source metadata should serialize")),
        last_scheduled_refresh: None,
    }
}

fn sample_media() -> medias::Model {
    let metadata = MediaMetadata {
        title: "Test Video".to_string(),
        description: None,
        duration: DURATION_SECONDS,
        extractor_key: "Youtube".to_string(),
        original_url: "https://example.com/watch?v=1".to_string(),
        timestamp: UPLOAD_TIMESTAMP,
    };
    medias::Model {
        created_at: sample_timestamp(),
        updated_at: sample_timestamp(),
        id: 7,
        url: "https://example.com/watch?v=1".to_string(),
        source_id: 1,
        metadata: Some(serde_json::to_value(metadata).expect("media metadata should serialize")),
        media_path: Some("Test_Channel/Test_Video.mkv".to_string()),
    }
}

async fn body_text(response: Response) -> String {
    let bytes = body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body should be readable");
    String::from_utf8(bytes.to_vec()).expect("response body should be UTF-8")
}

#[tokio::test]
async fn renders_media_list_with_whole_minutes_and_upload_date() {
    let view_engine = build_test_tera_engine().expect("TeraView build should succeed");
    let items = vec![(sample_media(), Some(sample_source()))];

    let response =
        views::media::list(&view_engine, &items).expect("Rendering media list view should succeed");
    let body = body_text(response).await;

    assert!(body.contains("Test Channel"), "uploader is missing: {body}");
    assert!(
        body.contains("</span> 46m</p>"),
        "duration is not in whole minutes: {body}"
    );
    assert!(
        body.contains("</span> 2026-09-30</p>"),
        "upload date is missing: {body}"
    );
}

#[tokio::test]
async fn renders_media_show_with_whole_minutes_and_upload_time() {
    let view_engine = build_test_tera_engine().expect("TeraView build should succeed");
    let source = sample_source();

    let response = views::media::show(&view_engine, &sample_media(), Some(&source))
        .expect("Rendering media show view should succeed");
    let body = body_text(response).await;

    assert!(
        body.contains("/medias/7/stream"),
        "player is missing: {body}"
    );
    assert!(
        body.contains("</span> 46m</p>"),
        "duration is not in whole minutes: {body}"
    );
    assert!(
        body.contains("</span> 2026-09-30 19:08:51</p>"),
        "upload time is missing: {body}"
    );
}

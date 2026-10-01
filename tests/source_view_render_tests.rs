use axum::body;
use chrono::DateTime;
use localtube::{
    initializers::view_engine::build_test_tera_engine,
    models::{_entities::sources, sources::SourceMetadata},
    views,
    ytdlp::SourceListTabOption,
};
use loco_rs::prelude::Response;

fn sample_timestamp() -> DateTime<chrono::FixedOffset> {
    DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z").expect("timestamp should parse")
}

fn sample_metadata_with_unknown_tab_count() -> SourceMetadata {
    SourceMetadata {
        uploader: "Test Channel".to_string(),
        items: 0,
        source_provider: "youtube".to_string(),
        list_kind: None,
        list_count: None,
        list_order: None,
        list_tab: Some("https://example.com/tab".to_string()),
        list_tabs: Some(vec![SourceListTabOption {
            url: "https://example.com/tab".to_string(),
            label: "Videos".to_string(),
        }]),
    }
}

fn sample_source(metadata: Option<SourceMetadata>) -> sources::Model {
    let timestamp = sample_timestamp();
    sources::Model {
        created_at: timestamp,
        updated_at: timestamp,
        id: 1,
        url: "https://example.com/channel".to_string(),
        fetch_last_days: 7,
        last_refreshed_at: None,
        refresh_frequency: 24,
        sponsorblock: "sponsor".to_string(),
        metadata: metadata
            .map(|data| serde_json::to_value(data).expect("metadata should serialize")),
        last_scheduled_refresh: None,
    }
}

/// Returns the response body with each whitespace run replaced by one space.
async fn collapsed_body_text(response: Response) -> String {
    let bytes = body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body should be readable");
    let text = String::from_utf8(bytes.to_vec()).expect("response body should be UTF-8");
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[test]
fn renders_source_list_with_unknown_tab_count() {
    let view_engine = build_test_tera_engine().expect("TeraView build should succeed");
    let source = sample_source(Some(sample_metadata_with_unknown_tab_count()));
    let sources = vec![source];

    views::source::list(&view_engine, &sources).expect("Rendering source list view should succeed");
}

#[test]
fn renders_source_show_with_unknown_tab_count() {
    let view_engine = build_test_tera_engine().expect("TeraView build should succeed");
    let source = sample_source(Some(sample_metadata_with_unknown_tab_count()));

    views::source::show(&view_engine, &source).expect("Rendering source show view should succeed");
}

#[tokio::test]
async fn renders_source_create_form_with_unchecked_sponsorblock_categories() {
    let view_engine = build_test_tera_engine().expect("TeraView build should succeed");

    let response =
        views::source::create(&view_engine).expect("Rendering source create view should succeed");
    let body = collapsed_body_text(response).await;

    assert!(
        body.contains(r#"name="sponsorblock_sponsor" class="sponsorblock-category mr-2" >"#),
        "sponsor checkbox must be present and unchecked: {body}"
    );
    assert!(
        body.contains(r#"<option value="4" selected>4h</option>"#),
        "default refresh frequency must be selected: {body}"
    );
    assert!(
        body.contains("function updateSponsorblock()"),
        "sponsorblock script is missing: {body}"
    );
}

#[tokio::test]
async fn renders_source_edit_form_with_saved_sponsorblock_categories() {
    let view_engine = build_test_tera_engine().expect("TeraView build should succeed");
    let source = sample_source(Some(sample_metadata_with_unknown_tab_count()));

    let response = views::source::edit(&view_engine, &source)
        .expect("Rendering source edit view should succeed");
    let body = collapsed_body_text(response).await;

    assert!(
        body.contains(r#"name="sponsorblock_sponsor" class="sponsorblock-category mr-2" checked>"#),
        "saved sponsor category must be checked: {body}"
    );
    assert!(
        body.contains(r#"name="sponsorblock_intro" class="sponsorblock-category mr-2" >"#),
        "unsaved intro category must be unchecked: {body}"
    );
    assert!(
        body.contains(r#"<option value="https://example.com/tab" selected>Videos</option>"#),
        "saved tab must be selected: {body}"
    );
    assert!(
        body.contains("function updateSponsorblock()"),
        "sponsorblock script is missing: {body}"
    );
}

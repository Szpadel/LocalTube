use chrono::DateTime;
use localtube::{
    initializers::view_engine::build_test_tera_engine,
    models::{_entities::sources, sources::SourceMetadata},
    views,
    ytdlp::SourceListTabOption,
};

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

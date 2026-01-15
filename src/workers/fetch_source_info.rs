use loco_rs::prelude::*;
use sea_orm::{Condition, Set};
use serde::{Deserialize, Serialize};
use tracing::{error, info, warn};

use crate::{
    job_tracking::{manager::register_refresh_task, task::ActiveTask},
    models::medias::MediaMetadata,
    workers::fetch_media::{FetchMediaWorker, FetchMediaWorkerArgs},
    ytdlp::{self, ListProbeMode, MediaListOrder, SourceListKind, SourceListOrder},
};
use crate::{
    models::{
        _entities::{
            medias::ActiveModel as MediaActiveModel, sources::ActiveModel as SourceActiveModel,
        },
        sources::SourceMetadata,
    },
    ytdlp::{probe_list_metadata, probe_list_tabs, stream_media_list, SourceListTabOption},
};

const SMALL_LIST_THRESHOLD: u64 = 25;

fn probe_mode_for(metadata: Option<&SourceMetadata>) -> ListProbeMode {
    match metadata {
        None => ListProbeMode::OrderAware,
        Some(meta) => match meta.list_kind.as_ref() {
            Some(SourceListKind::Video) => ListProbeMode::Minimal,
            Some(SourceListKind::List) => {
                if meta.list_order.is_some() {
                    // Order is already known; keep it stable and just refresh counts.
                    ListProbeMode::Minimal
                } else {
                    // We do not know ordering yet; probe two items to infer it once.
                    ListProbeMode::OrderAware
                }
            }
            None => ListProbeMode::OrderAware,
        },
    }
}

fn should_stop_early(list_kind: Option<&SourceListKind>, list_count: Option<u64>) -> bool {
    match list_kind {
        Some(SourceListKind::List) => {
            // Small lists are safe to scan fully; large lists need early stop.
            !matches!(list_count, Some(count) if count <= SMALL_LIST_THRESHOLD)
        }
        _ => true,
    }
}

fn stream_order_for(
    list_kind: Option<&SourceListKind>,
    list_order: Option<&SourceListOrder>,
) -> MediaListOrder {
    match list_kind {
        Some(SourceListKind::List) => match list_order {
            Some(SourceListOrder::OldestFirst) => MediaListOrder::Reverse,
            _ => MediaListOrder::Original,
        },
        _ => MediaListOrder::Original,
    }
}

fn normalize_tab_value(value: &str) -> String {
    let trimmed = value.trim();
    let path = strip_query_fragment(trimmed).trim_end_matches('/');
    if matches_known_tab_suffix(path) {
        path.to_string()
    } else {
        trimmed.trim_end_matches('/').to_string()
    }
}

fn strip_query_fragment(value: &str) -> &str {
    value.split(['?', '#']).next().unwrap_or(value)
}

fn matches_known_tab_suffix(value: &str) -> bool {
    value.ends_with("/videos")
        || value.ends_with("/streams")
        || value.ends_with("/shorts")
        || value.ends_with("/playlists")
}

fn strip_known_tab_suffix(path: &str) -> &str {
    for suffix in ["/videos", "/streams", "/shorts", "/playlists"] {
        if let Some(stripped) = path.strip_suffix(suffix) {
            return stripped;
        }
    }
    path
}

fn tab_matches_source(tab_url: &str, source_url: &str) -> bool {
    let tab_path = strip_query_fragment(tab_url).trim_end_matches('/');
    let source_path = strip_query_fragment(source_url).trim_end_matches('/');
    if matches_known_tab_suffix(tab_path) || matches_known_tab_suffix(source_path) {
        strip_known_tab_suffix(tab_path) == strip_known_tab_suffix(source_path)
    } else {
        normalize_tab_value(tab_url) == normalize_tab_value(source_url)
    }
}

fn resolve_selected_tab(
    existing: Option<&SourceMetadata>,
    tabs: &[SourceListTabOption],
) -> Option<String> {
    let selected = existing.and_then(|m| m.list_tab.as_ref())?;
    let selected_norm = normalize_tab_value(selected);
    tabs.iter()
        .find(|tab| normalize_tab_value(&tab.url) == selected_norm)
        .map(|tab| tab.url.clone())
}
pub struct FetchSourceInfoWorker {
    pub ctx: AppContext,
}

#[derive(Deserialize, Debug, Serialize)]
pub struct FetchSourceInfoWorkerArgs {
    pub source_id: i32,
}

impl FetchSourceInfoWorker {
    /// Schedules a source refresh job while updating the `last_scheduled_refresh` timestamp.
    ///
    /// # Errors
    ///
    /// Returns an error if updating the source record or enqueueing the job fails.
    pub async fn schedule_refresh(ctx: &AppContext, source_id: i32) -> Result<()> {
        // Update the last_scheduled_refresh timestamp before scheduling the job
        let source_update = SourceActiveModel {
            id: Set(source_id),
            last_scheduled_refresh: Set(Some(chrono::Utc::now())),
            ..Default::default()
        };
        crate::models::sources::Sources::update(source_update)
            .exec(&ctx.db)
            .await?;

        // Now schedule the actual job
        Self::perform_later(ctx, FetchSourceInfoWorkerArgs { source_id }).await?;
        Ok(())
    }
}

#[async_trait]
impl BackgroundWorker<FetchSourceInfoWorkerArgs> for FetchSourceInfoWorker {
    fn build(ctx: &AppContext) -> Self {
        Self { ctx: ctx.clone() }
    }
    #[allow(clippy::too_many_lines)]
    async fn perform(&self, args: FetchSourceInfoWorkerArgs) -> Result<()> {
        // Store ActiveTask (not queued)
        let mut task: Option<ActiveTask> = None;

        // Try to execute the source info fetching operation
        let result = async {
            let source = crate::models::sources::Sources::find_by_id(args.source_id)
                .one(&self.ctx.db)
                .await
                .map_err(Box::from)?;

            if let Some(source) = source {
                info!("Fetching source info for {}", source.url);

                // Register the task with the TaskManager
                let task_title = format!(
                    "Refreshing {}",
                    source
                        .get_metadata()
                        .map_or_else(|| source.url.clone(), |m| m.uploader.clone())
                );

                // Register task as Queued
                let queued = register_refresh_task(task_title);

                // Acquire semaphore and transition to Active
                // This is where the task actually waits if semaphore is full!
                let active = queued
                    .start(crate::ytdlp::ytdtp_concurrency().clone())
                    .await;
                active.update_status("Fetching channel metadata...".to_string());

                task = Some(active);

                let existing_metadata = source.get_metadata();
                let list_tabs = match probe_list_tabs(&source.url).await {
                    Ok(tabs) => tabs,
                    Err(err) => {
                        // Tab probing is best-effort; preserve prior tabs on transient failures.
                        warn!(error = %err, "failed to probe list tabs");
                        existing_metadata
                            .as_ref()
                            .and_then(|m| m.list_tabs.clone())
                            .unwrap_or_default()
                    }
                };
                let has_tabs = !list_tabs.is_empty();
                let url_tab_option = list_tabs
                    .iter()
                    .find(|tab| normalize_tab_value(&tab.url) == normalize_tab_value(&source.url))
                    .map(|tab| tab.url.clone());
                let source_url_matches_tab = url_tab_option.is_some();
                let url_tab = url_tab_option;
                // Only reuse a stored tab when it still matches the current source URL.
                let stored_tab = existing_metadata
                    .as_ref()
                    .and_then(|m| m.list_tab.clone())
                    .filter(|tab| tab_matches_source(tab, &source.url));
                let selected_tab = if has_tabs {
                    let resolved_tab = resolve_selected_tab(existing_metadata.as_ref(), &list_tabs);
                    match (resolved_tab, url_tab.clone()) {
                        (Some(stored), _) => Some(stored),
                        (None, Some(url)) => Some(url),
                        (None, None) => None,
                    }
                } else {
                    stored_tab.clone()
                };
                // Avoid reusing cached counts/order when the effective tab changed.
                let tab_changed = selected_tab.as_ref().map(|tab| normalize_tab_value(tab))
                    != stored_tab.as_ref().map(|tab| normalize_tab_value(tab));
                let list_tabs = if has_tabs { Some(list_tabs) } else { None };
                let list_tab = if has_tabs {
                    selected_tab.clone()
                } else {
                    stored_tab
                };
                let effective_url = match selected_tab.as_ref() {
                    Some(selected)
                        if source_url_matches_tab
                            && normalize_tab_value(selected)
                                == normalize_tab_value(&source.url) =>
                    {
                        // Preserve query/fragment for tabbed URLs while storing canonical tab.
                        source.url.clone()
                    }
                    Some(selected) => selected.clone(),
                    None => source.url.clone(),
                };
                let probe_mode = probe_mode_for(existing_metadata.as_ref());
                // Delegate list detection to yt-dlp so all providers stay supported.
                let probe = probe_list_metadata(&effective_url, probe_mode)
                    .await
                    .map_err(|e| Error::string(&format!("Failed to probe source metadata: {e}")))?;
                let list_kind = Some(probe.list_kind);
                let mut list_count = probe.list_count.or_else(|| {
                    if tab_changed {
                        None
                    } else {
                        existing_metadata.as_ref().and_then(|m| m.list_count)
                    }
                });
                if has_tabs && selected_tab.is_none() {
                    // Tab containers report the number of tabs, not the number of videos.
                    list_count = None;
                }
                // Once order is known, keep it stable across refreshes.
                let list_order = probe.list_order.or_else(|| {
                    if tab_changed {
                        None
                    } else {
                        existing_metadata
                            .as_ref()
                            .and_then(|m| m.list_order.clone())
                    }
                });
                let order_known = list_order.is_some();
                let uploader = probe
                    .uploader
                    .or_else(|| existing_metadata.as_ref().map(|m| m.uploader.clone()))
                    .unwrap_or_else(|| source.url.clone());
                let source_provider = probe
                    .source_provider
                    .or_else(|| {
                        existing_metadata
                            .as_ref()
                            .map(|m| m.source_provider.clone())
                    })
                    .unwrap_or_else(|| "unknown".to_string());
                // For single videos, enforce a count of 1 to keep UI consistent.
                let items = match list_kind {
                    Some(SourceListKind::Video) => 1,
                    Some(SourceListKind::List) => list_count
                        .or_else(|| {
                            if tab_changed || (has_tabs && selected_tab.is_none()) {
                                Some(0)
                            } else {
                                existing_metadata.as_ref().map(|m| m.items)
                            }
                        })
                        .unwrap_or(0),
                    None => existing_metadata.as_ref().map_or(0, |m| m.items),
                };

                let source_metadata = SourceMetadata {
                    uploader,
                    items,
                    source_provider,
                    list_kind,
                    list_count,
                    list_order,
                    list_tab,
                    list_tabs,
                };

                let source_update = SourceActiveModel {
                    id: Set(source.id),
                    metadata: Set(Some(
                        serde_json::to_value(source_metadata.clone())
                            .map_err(|_| Error::string("Failed to serialize source metadata"))?,
                    )),
                    ..Default::default()
                };
                crate::models::sources::Sources::update(source_update)
                    .exec(&self.ctx.db)
                    .await?;

                if let Some(task) = &task {
                    task.update_status("Fetching video list...".to_string());
                }

                let fetch_before_timestamp = chrono::Utc::now()
                    .checked_sub_signed(chrono::Duration::days(i64::from(source.fetch_last_days)))
                    .unwrap()
                    .timestamp();

                let should_stop_early = should_stop_early(
                    source_metadata.list_kind.as_ref(),
                    source_metadata.list_count,
                );
                let stream_order = stream_order_for(
                    source_metadata.list_kind.as_ref(),
                    source_metadata.list_order.as_ref(),
                );
                let mut media_stream = stream_media_list(&effective_url, stream_order).await;
                let mut media_count = 0;
                let mut saw_newer_item = false;

                while let Some(item) = media_stream.recv().await {
                    let metadata = match item {
                        Ok(metadata) => metadata,
                        Err(err) => return Err(err),
                    };
                    media_count += 1;

                    if let Some(task) = &task {
                        task.update_status(format!(
                            "Processing video {} ({})",
                            media_count, metadata.title
                        ));
                    }

                    let mut download_media_id = None;
                    info!(
                        "{}: Fetching media info for {}",
                        &source_metadata.uploader, &metadata.title
                    );
                    if metadata.timestamp >= fetch_before_timestamp {
                        saw_newer_item = true;
                    }
                    if should_stop_early
                        && metadata.timestamp < fetch_before_timestamp
                        && (order_known || saw_newer_item)
                    {
                        // For unknown order, avoid stopping before we see any recent items.
                        break;
                    }

                    // try to find existing media by url
                    let media = crate::models::medias::Medias::find()
                        .filter(
                            Condition::all()
                                .add(
                                    crate::models::_entities::medias::Column::SourceId
                                        .eq(source.id),
                                )
                                .add(
                                    crate::models::_entities::medias::Column::Url
                                        .contains(&metadata.original_url),
                                ),
                        )
                        .one(&self.ctx.db)
                        .await
                        .map_err(Box::from)?;

                    let media_metadata: MediaMetadata = metadata.into();
                    if let Some(media) = media {
                        if media.media_path.is_none() {
                            download_media_id = Some(media.id);
                        }

                        let mut media_update = MediaActiveModel {
                            id: Set(media.id),
                            metadata: Set(Some(
                                serde_json::to_value(media_metadata.clone()).map_err(Error::msg)?,
                            )),
                            ..Default::default()
                        };

                        if let Some(media_path) = &media.media_path {
                            if !ytdlp::media_directory().join(media_path).exists() {
                                warn!(
                                    "{}: Media file not found for {} expected file in {}",
                                    &source_metadata.uploader, &media_metadata.title, media_path
                                );
                                media_update.media_path = Set(None);
                                download_media_id = Some(media.id);
                            }
                        }
                        crate::models::medias::Medias::update(media_update)
                            .exec(&self.ctx.db)
                            .await?;
                    } else {
                        let media_insert = MediaActiveModel {
                            source_id: Set(source.id),
                            url: Set(media_metadata.original_url.clone()),
                            metadata: Set(Some(
                                serde_json::to_value(media_metadata).map_err(Error::msg)?,
                            )),
                            ..Default::default()
                        };
                        let media = crate::models::medias::Medias::insert(media_insert)
                            .exec(&self.ctx.db)
                            .await?;
                        download_media_id = Some(media.last_insert_id);
                    }
                    if let Some(media_id) = download_media_id {
                        FetchMediaWorker::perform_later(
                            &self.ctx,
                            FetchMediaWorkerArgs { media_id },
                        )
                        .await?;
                    }
                }

                if let Some(task) = &task {
                    task.update_status("Cleaning up old videos...".to_string());
                }

                // select all media that were created after the fetch_before_timestamp
                // this info is stored in metadata.timestamp, so we need to load all media for source in batches and check the timestamp

                let medias = crate::models::medias::Medias::find()
                    .filter(crate::models::_entities::medias::Column::SourceId.eq(source.id))
                    .all(&self.ctx.db)
                    .await?;

                for media in medias {
                    if let Some(metadata) = media.get_metadata() {
                        if metadata.timestamp < fetch_before_timestamp && media.media_path.is_some()
                        {
                            info!(
                                "{}: Removing old media {}",
                                &source_metadata.uploader, &metadata.title
                            );
                            media.remove_media_files()?;
                            media.delete(&self.ctx.db).await?;
                        }
                    }
                }

                let source_update = SourceActiveModel {
                    id: Set(source.id),
                    last_refreshed_at: Set(Some(chrono::Utc::now())),
                    ..Default::default()
                };
                crate::models::sources::Sources::update(source_update)
                    .exec(&self.ctx.db)
                    .await?;

                info!("{}: Finished source reindex", source_metadata.uploader);
            }

            Ok(())
        }
        .await;

        // Handle errors if any
        if let Err(e) = &result {
            error!("Source refresh failed: {}", e);

            // Report the error if we have a task
            if let Some(t) = task.take() {
                let error_msg = match e {
                    Error::Message(msg) => msg.clone(),
                    _ => format!(
                        "Source refresh failed: {}",
                        e.to_string().split('\n').next().unwrap_or("Unknown error")
                    ),
                };
                t.mark_failed(error_msg);
            }
        } else {
            // On success, mark the task as complete for metrics
            if let Some(t) = task.take() {
                t.complete();
            }
        }

        // Return the original result to propagate errors properly
        result
    }
}

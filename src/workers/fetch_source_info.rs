use loco_rs::prelude::*;
use sea_orm::Condition;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::{
    models::medias::MediaMetadata,
    workers::fetch_media::{FetchMediaWorker, FetchMediaWorkerArgs},
    ytdlp::{self, download_last_video_metadata},
};
use crate::{
    models::{
        _entities::{
            medias::ActiveModel as MediaActiveModel, sources::ActiveModel as SourceActiveModel,
        },
        sources::SourceMetadata,
    },
    ytdlp::stream_media_list,
};
pub struct FetchSourceInfoWorker {
    pub ctx: AppContext,
}

#[derive(Deserialize, Debug, Serialize)]
pub struct FetchSourceInfoWorkerArgs {
    pub source_id: i32,
}

#[async_trait]
impl BackgroundWorker<FetchSourceInfoWorkerArgs> for FetchSourceInfoWorker {
    fn build(ctx: &AppContext) -> Self {
        Self { ctx: ctx.clone() }
    }
    async fn perform(&self, args: FetchSourceInfoWorkerArgs) -> Result<()> {
        let source = crate::models::sources::Sources::find_by_id(args.source_id)
            .one(&self.ctx.db)
            .await
            .map_err(Box::from)?;

        if let Some(source) = source {
            info!("Fetching source info for {}", source.url);
            let metadata = download_last_video_metadata(&source.url)
                .await
                .map_err(Error::msg)?;
            let source_metadata: SourceMetadata = metadata.into();

            let source_update = SourceActiveModel {
                id: Set(source.id),
                metadata: Set(Some(
                    serde_json::to_value(source_metadata.clone()).map_err(Error::msg)?,
                )),
                ..Default::default()
            };
            crate::models::sources::Sources::update(source_update)
                .exec(&self.ctx.db)
                .await?;

            let fetch_before_timestamp = chrono::Utc::now()
                .checked_sub_signed(chrono::Duration::days(i64::from(source.fetch_last_days)))
                .unwrap()
                .timestamp();

            let mut media_stream = stream_media_list(&source.url).await;
            while let Some(metadata) = media_stream.recv().await {
                let mut download_media_id = None;
                info!(
                    "{}: Fetching media info for {}",
                    &source_metadata.uploader, &metadata.title
                );
                if metadata.timestamp < fetch_before_timestamp {
                    break;
                }

                // try to find existing media by url
                let media = crate::models::medias::Medias::find()
                    .filter(
                        Condition::all()
                            .add(crate::models::_entities::medias::Column::SourceId.eq(source.id))
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
                    FetchMediaWorker::perform_later(&self.ctx, FetchMediaWorkerArgs { media_id })
                        .await?;
                }
            }

            // select all media that were created after the fetch_before_timestamp
            // this info is stored in metadata.timestamp, so we need to load all media for source in batches and check the timestamp

            let medias = crate::models::medias::Medias::find()
                .filter(crate::models::_entities::medias::Column::SourceId.eq(source.id))
                .all(&self.ctx.db)
                .await?;

            for media in medias {
                if let Some(metadata) = media.get_metadata() {
                    if metadata.timestamp < fetch_before_timestamp && media.media_path.is_some() {
                        info!(
                            "{}: Removing old media {}",
                            &source_metadata.uploader, &metadata.title
                        );
                        if let Some(path) = &media.media_path {
                            let base_path = ytdlp::media_directory().join(path);
                            let info_path = base_path.with_extension("info.json");
                            for path in [&info_path, &base_path] {
                                if path.exists() {
                                    std::fs::remove_file(path).map_err(|e| {
                                        Error::string(&format!("Failed to remove file {}: {}", path.display(), e))
                                    })?;
                                }
                            }
                        }
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
            info!(
                "{}: Finished source reindex",
                source_metadata.uploader
            );
        }

        Ok(())
    }
}
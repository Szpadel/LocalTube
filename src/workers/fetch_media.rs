use loco_rs::prelude::*;
use serde::{Deserialize, Serialize};
use tracing::{error, info};

pub struct FetchMediaWorker {
    pub ctx: AppContext,
}

#[derive(Deserialize, Debug, Serialize)]
pub struct FetchMediaWorkerArgs {
    pub media_id: i32,
}

#[async_trait]
impl BackgroundWorker<FetchMediaWorkerArgs> for FetchMediaWorker {
    fn build(ctx: &AppContext) -> Self {
        Self { ctx: ctx.clone() }
    }
    async fn perform(&self, args: FetchMediaWorkerArgs) -> Result<()> {
        // We'll use this variable to store task ID for error reporting
        let mut task_id: Option<String> = None;

        // Try to execute the download operation
        let result = async {
            let media = crate::models::medias::Medias::find_by_id(args.media_id)
                .one(&self.ctx.db)
                .await
                .map_err(Box::from)?;
            if media.is_none() {
                return Ok(());
            }
            let media = media.unwrap();
            if media.media_path.is_some() {
                return Ok(());
            }

            let metadata = media.get_metadata();
            if metadata.is_none() {
                return Ok(());
            }
            let metadata = metadata.unwrap();

            let source = media
                .find_related(crate::models::_entities::sources::Entity)
                .one(&self.ctx.db)
                .await?;
            if source.is_none() {
                // TODO: delete media
                return Ok(());
            }
            let source = source.unwrap();

            let source_metadata = source.get_metadata();
            if source_metadata.is_none() {
                return Ok(());
            }
            let source_metadata = source_metadata.unwrap();

            // Register the task with the TaskManager
            task_id = Some(crate::ws::register_download_task(metadata.title.clone()));

            info!(
                "{}: Downloading {}",
                &source_metadata.source_provider, &metadata.title,
            );

            // This is where errors are most likely to happen
            let file_path = crate::ytdlp::download_media(&metadata.original_url, &source)
                .await
                .map_err(Error::msg)?;

            info!(
                "{} Downloaded {} to {}",
                &source_metadata.source_provider, &metadata.title, file_path
            );

            let media_update = crate::models::_entities::medias::ActiveModel {
                id: Set(media.id),
                media_path: Set(Some(file_path)),
                ..Default::default()
            };
            crate::models::medias::Medias::update(media_update)
                .exec(&self.ctx.db)
                .await?;

            // Mark the task as successfully completed
            if let Some(id) = &task_id {
                crate::ws::TaskManager::global().remove_task(id);
                task_id = None; // Clear ID so we don't process it again in error handling
            }

            Ok(())
        }
        .await;

        // Handle errors if any - we only want to report the failure, not change the result
        if let Err(e) = &result {
            error!("Download failed: {}", e);

            // Report the error if we have a task ID
            if let Some(id) = &task_id {
                crate::ws::TaskManager::global()
                    .mark_task_failed(id, format!("Download failed: {}", e));
            }
        }

        // Return the original result to propagate errors properly
        result
    }
}

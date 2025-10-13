use loco_rs::prelude::*;
use serde::{Deserialize, Serialize};
use tracing::{error, info};

use crate::job_tracking::{manager::register_download_task, task::ActiveTask};

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
        // Store ActiveTask (not queued)
        let mut task: Option<ActiveTask> = None;

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

            // Register task as Queued
            let queued = register_download_task(metadata.title.clone());

            // Acquire semaphore and transition to Active
            // This is where the task actually waits if semaphore is full!
            let active = queued
                .start(crate::ytdlp::ytdtp_concurrency().clone())
                .await;
            active.update_status("Downloading...".to_string());

            task = Some(active);

            info!(
                "{}: Downloading {}",
                &source_metadata.source_provider, &metadata.title,
            );

            // This is where errors are most likely to happen
            let file_path = crate::ytdlp::download_media(&metadata.original_url, &source)
                .await
                .map_err(|e| Error::string(&format!("Download failed: {e}")))?;

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

            Ok(())
        }
        .await;

        // Handle errors if any - only mark failed if we still have the task
        if let Err(e) = &result {
            error!("Download failed: {}", e);

            // Report the error if we have a task
            if let Some(t) = task.take() {
                let error_msg = match e {
                    Error::Message(msg) => msg.clone(),
                    _ => format!(
                        "Download failed: {}",
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

use loco_rs::prelude::*;
use serde::{Deserialize, Serialize};
use tracing::info;

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
        let task_id = crate::ws::register_download_task(metadata.title.clone());

        info!(
            "{}: Downloading {}",
            &source_metadata.source_provider, &metadata.title,
        );
        let file_path = crate::ytdlp::download_media(&metadata.original_url, &source)
            .await
            .map_err(Error::msg)?;
        info!(
            "{} Downloaded {} to {}",
            &source_metadata.source_provider, &metadata.title, file_path
        );

        // Remove the task from the TaskManager
        crate::ws::TaskManager::global().remove_task(&task_id);

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
}

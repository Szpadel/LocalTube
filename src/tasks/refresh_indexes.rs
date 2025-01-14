use loco_rs::prelude::*;

use crate::workers::fetch_source_info::{FetchSourceInfoWorker, FetchSourceInfoWorkerArgs};

pub struct RefreshIndexes;
#[async_trait]
impl Task for RefreshIndexes {
    fn task(&self) -> TaskInfo {
        TaskInfo {
            name: "refresh_indexes".to_string(),
            detail: "Refresh source indexes".to_string(),
        }
    }
    async fn run(&self, ctx: &AppContext, vars: &task::Vars) -> Result<()> {
        let sources = crate::models::sources::Sources::find().all(&ctx.db).await?;

        for source in sources {
            let jitter = source
                .last_refreshed_at
                .map_or(0, |last_refresh| (last_refresh.timestamp() % 1800) - 900);

            let need_refresh = source.last_refreshed_at.is_none_or(|d| {
                (chrono::Utc::now() - d).num_seconds()
                    > i64::from(source.refresh_frequency * 3600) + jitter
            });

            if source.get_metadata().is_none() || need_refresh || vars.cli_arg("force").is_ok() {
                FetchSourceInfoWorker::perform_later(
                    ctx,
                    FetchSourceInfoWorkerArgs {
                        source_id: source.id,
                    },
                )
                .await?;
            }
        }
        Ok(())
    }
}

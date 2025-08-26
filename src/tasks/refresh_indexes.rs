use loco_rs::prelude::*;

use crate::workers::fetch_source_info::FetchSourceInfoWorker;

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

            let is_time_passed = |timestamp: Option<chrono::DateTime<chrono::Utc>>| -> bool {
                timestamp.is_none_or(|d| {
                    (chrono::Utc::now() - d).num_seconds()
                        > i64::from(source.refresh_frequency * 3600) + jitter
                })
            };

            let need_refresh = is_time_passed(source.last_refreshed_at);
            let need_schedule = is_time_passed(source.last_scheduled_refresh);

            if (source.get_metadata().is_none() || need_refresh) && need_schedule
                || vars.cli_arg("force").is_ok()
            {
                FetchSourceInfoWorker::schedule_refresh(ctx, source.id).await?;
            }
        }
        Ok(())
    }
}

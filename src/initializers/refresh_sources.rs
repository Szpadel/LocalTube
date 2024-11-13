use std::vec;

use axum::async_trait;
use loco_rs::{
    app::{AppContext, Initializer},
    task::Task,
    Result,
};
use tracing::error;

use crate::tasks::refresh_indexes::RefreshIndexes;

pub struct RefreshSources;

#[async_trait]
impl Initializer for RefreshSources {
    fn name(&self) -> String {
        "refresh-sources".to_string()
    }

    async fn before_run(&self, ctx: &AppContext) -> Result<()> {
        if let Err(e) = (RefreshIndexes)
            .run(
                ctx,
                &loco_rs::task::Vars::from_cli_args(vec![(
                    "force".to_string(),
                    "true".to_string(),
                )]),
            )
            .await
        {
            error!("RefreshIndexes error: {:?}", e);
        }

        let ctx = ctx.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(tokio::time::Duration::from_secs(300)).await;

                if let Err(e) = (RefreshIndexes)
                    .run(&ctx, &loco_rs::task::Vars::default())
                    .await
                {
                    error!("RefreshIndexes error: {:?}", e);
                }
            }
        });

        Ok(())
    }
}

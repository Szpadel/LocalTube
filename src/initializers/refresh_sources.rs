use axum::async_trait;
use loco_rs::{
    app::{AppContext, Initializer},
    task::Task,
    Result,
};

use crate::tasks::refresh_indexes::RefreshIndexes;

pub struct RefreshSources;

#[async_trait]
impl Initializer for RefreshSources {
    fn name(&self) -> String {
        "refresh-sources".to_string()
    }

    async fn before_run(&self, ctx: &AppContext) -> Result<()> {
        (RefreshIndexes).run(ctx, &loco_rs::task::Vars::default()).await?;

        Ok(())
    }
}

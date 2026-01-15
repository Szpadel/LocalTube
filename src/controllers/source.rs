#![allow(clippy::missing_errors_doc)]
#![allow(clippy::unnecessary_struct_initialization)]
#![allow(clippy::unused_async)]
use axum::debug_handler;
use loco_rs::prelude::*;
use sea_orm::{sea_query::Order, QueryOrder};
use serde::{Deserialize, Serialize};

use crate::{
    models::_entities::sources::{ActiveModel, Column, Entity, Model},
    views,
    workers::fetch_source_info::{FetchSourceInfoWorker, FetchSourceInfoWorkerArgs},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Params {
    pub url: Option<String>,
    pub fetch_last_days: i32,
    pub sponsorblock: String,
    pub refresh_frequency: i32,
    pub list_tab: Option<String>,
}

impl Params {
    fn update(&self, item: &mut ActiveModel) {
        if let Some(url) = &self.url {
            item.url = Set(url.clone());
        }
        item.fetch_last_days = Set(self.fetch_last_days);
        item.sponsorblock = Set(self.sponsorblock.clone());
        item.refresh_frequency = Set(self.refresh_frequency);
    }
}

fn normalize_list_tab(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("auto") {
        None
    } else {
        Some(trimmed.trim_end_matches('/').to_string())
    }
}

async fn load_item(ctx: &AppContext, id: i32) -> Result<Model> {
    let item = Entity::find_by_id(id).one(&ctx.db).await?;
    item.ok_or_else(|| Error::NotFound)
}

#[debug_handler]
pub async fn list(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
) -> Result<Response> {
    let item = Entity::find()
        .order_by(Column::Id, Order::Desc)
        .all(&ctx.db)
        .await?;
    views::source::list(&v, &item)
}

#[debug_handler]
pub async fn new(
    ViewEngine(v): ViewEngine<TeraView>,
    State(_ctx): State<AppContext>,
) -> Result<Response> {
    views::source::create(&v)
}

#[debug_handler]
pub async fn update(
    Path(id): Path<i32>,
    State(ctx): State<AppContext>,
    Json(params): Json<Params>,
) -> Result<Response> {
    let model = load_item(&ctx, id).await?;
    let mut item = model.clone().into_active_model();
    params.update(&mut item);
    let url_changed = params.url.as_ref().is_some_and(|url| url != &model.url);
    if let Some(mut metadata) = model.get_metadata() {
        let mut metadata_changed = false;
        if url_changed {
            metadata.list_tab = None;
            metadata.list_tabs = None;
            metadata.items = 0;
            metadata.list_order = None;
            metadata.list_count = None;
            metadata_changed = true;
        }
        if let Some(list_tab) = params.list_tab.as_ref() {
            let normalized = normalize_list_tab(list_tab);
            let changed = metadata.list_tab != normalized;
            if changed {
                // Clearing cached order/count forces a fresh probe for the new tab.
                metadata.list_tab = normalized;
                metadata.items = 0;
                metadata.list_order = None;
                metadata.list_count = None;
                metadata_changed = true;
            }
        }
        if metadata_changed {
            item.metadata =
                Set(Some(serde_json::to_value(metadata).map_err(|_| {
                    Error::string("Failed to serialize source metadata")
                })?));
        }
    }
    let item = item.update(&ctx.db).await?;
    FetchSourceInfoWorker::perform_later(&ctx, FetchSourceInfoWorkerArgs { source_id: item.id })
        .await?;
    format::html(
        "<div class=\"text-sm text-green-600 dark:text-green-400\">Saved. Refresh queued.</div>",
    )
}

#[debug_handler]
pub async fn edit(
    Path(id): Path<i32>,
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
) -> Result<Response> {
    let item = load_item(&ctx, id).await?;
    views::source::edit(&v, &item)
}

#[debug_handler]
pub async fn show(
    Path(id): Path<i32>,
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
) -> Result<Response> {
    let item = load_item(&ctx, id).await?;
    views::source::show(&v, &item)
}

#[debug_handler]
pub async fn add(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
    Json(params): Json<Params>,
) -> Result<Response> {
    let mut item = ActiveModel {
        ..Default::default()
    };
    params.update(&mut item);
    let item = item.insert(&ctx.db).await?;
    FetchSourceInfoWorker::perform_later(&ctx, FetchSourceInfoWorkerArgs { source_id: item.id })
        .await?;
    views::source::show(&v, &item)
}

#[debug_handler]
pub async fn remove(Path(id): Path<i32>, State(ctx): State<AppContext>) -> Result<Response> {
    load_item(&ctx, id).await?.delete(&ctx.db).await?;
    format::empty()
}

pub fn routes() -> Routes {
    Routes::new()
        .prefix("sources/")
        .add("/", get(list))
        .add("/", post(add))
        .add("new", get(new))
        .add("{id}", get(show))
        .add("{id}/edit", get(edit))
        .add("{id}", delete(remove))
        .add("{id}", put(update))
        .add("{id}", post(update))
        .add("{id}", patch(update))
}

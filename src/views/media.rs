use loco_rs::prelude::*;

use crate::models::_entities::{medias, sources};

/// Render a list view of medias.
///
/// # Errors
///
/// When there is an issue with rendering the view.
pub fn list(
    v: &impl ViewRenderer,
    items: &[(medias::Model, Option<sources::Model>)],
) -> Result<Response> {
    format::render().view(v, "media/list.html", data!({"items": items}))
}

/// Render a single media view.
///
/// # Errors
///
/// When there is an issue with rendering the view.
pub fn show(
    v: &impl ViewRenderer,
    item: &medias::Model,
    source: Option<&sources::Model>,
) -> Result<Response> {
    format::render().view(
        v,
        "media/show.html",
        data!({
            "item": item,
            "source": source
        }),
    )
}

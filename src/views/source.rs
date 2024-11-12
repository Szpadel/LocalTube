use loco_rs::prelude::*;

use crate::models::_entities::sources;

/// Render a list view of sources.
///
/// # Errors
///
/// When there is an issue with rendering the view.
pub fn list(v: &impl ViewRenderer, items: &Vec<sources::Model>) -> Result<Response> {
    format::render().view(v, "source/list.html", data!({"items": items}))
}

/// Render a single source view.
///
/// # Errors
///
/// When there is an issue with rendering the view.
pub fn show(v: &impl ViewRenderer, item: &sources::Model) -> Result<Response> {
    format::render().view(v, "source/show.html", data!({"item": item}))
}

/// Render a source create form.
///
/// # Errors
///
/// When there is an issue with rendering the view.
pub fn create(v: &impl ViewRenderer) -> Result<Response> {
    format::render().view(v, "source/create.html", data!({}))
}

/// Render a source edit form.
///
/// # Errors
///
/// When there is an issue with rendering the view.
pub fn edit(v: &impl ViewRenderer, item: &sources::Model) -> Result<Response> {
    format::render().view(v, "source/edit.html", data!({"item": item}))
}

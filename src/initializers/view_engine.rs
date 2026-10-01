use async_trait::async_trait;
use axum::{Extension, Router as AxumRouter};
use fluent_templates::{ArcLoader, FluentLoader};
use loco_rs::{
    app::{AppContext, Initializer},
    controller::views::{engines, ViewEngine},
    environment::Environment,
    Error, Result,
};
use std::fmt::Write;
use std::path::{Path, PathBuf};
use tracing::info;

const I18N_DIR: &str = "assets/i18n";
// fluent-templates 0.15 loads each top-level `.ftl` file in `I18N_DIR` as a
// locale when its file stem is a valid language ID. `shared` is a valid
// language ID, so this file must stay outside `I18N_DIR`.
const I18N_SHARED: &str = "assets/shared.ftl";

pub struct ViewEngineInitializer;
#[async_trait]
impl Initializer for ViewEngineInitializer {
    fn name(&self) -> String {
        "view-engine".to_string()
    }

    async fn after_routes(&self, router: AxumRouter, ctx: &AppContext) -> Result<AxumRouter> {
        let tera_engine = match ctx.environment {
            Environment::Test => build_test_tera_engine()?,
            _ => engines::TeraView::build_with_post_process(configure_tera)?,
        };

        Ok(router.layer(Extension(ViewEngine::from(tera_engine))))
    }
}

/// Registers the filters and functions that the views call.
///
/// Tera 2 rejects a template that calls an unknown filter or function, so this
/// must run before the templates load.
fn configure_tera(tera: &mut tera::Tera) -> Result<()> {
    tera.register_filter("date", format_unix_timestamp);
    if Path::new(I18N_DIR).exists() {
        let arc = ArcLoader::builder(I18N_DIR, unic_langid::langid!("en-US"))
            .shared_resources(Some([I18N_SHARED.into()].as_slice()))
            .customize(|bundle| bundle.set_use_isolating(false))
            .build()
            .map_err(|e| Error::string(&e.to_string()))?;
        tera.register_function("t", FluentLoader::new(arc));
        info!("locales loaded");
    }
    Ok(())
}

/// Formats Unix seconds as UTC with the chrono `strftime` pattern in the `format` argument.
///
/// Tera 2 has no built-in `date` filter. This filter keeps the Tera 1 usage
/// `{{ timestamp | date(format="%Y-%m-%d") }}` for integer timestamps.
///
/// # Errors
///
/// Returns an error if `format` is missing or invalid, or if the timestamp is out of range.
#[allow(
    clippy::needless_pass_by_value,
    reason = "Tera 2 gives keyword arguments to filters by value"
)]
fn format_unix_timestamp(
    timestamp: i64,
    kwargs: tera::Kwargs,
    _: &tera::State<'_>,
) -> tera::TeraResult<String> {
    let format = kwargs.must_get::<&str>("format")?;
    let datetime = chrono::DateTime::from_timestamp(timestamp, 0)
        .ok_or_else(|| tera::Error::message(format!("Timestamp {timestamp} is out of range")))?;
    let mut formatted = String::new();
    write!(formatted, "{}", datetime.format(format))
        .map_err(|_| tera::Error::message(format!("Date format `{format}` is not valid")))?;
    Ok(formatted)
}

/// Builds the Tera view engine for test environments.
///
/// # Errors
///
/// Returns an error if view templates cannot be loaded.
pub fn build_test_tera_engine() -> Result<engines::TeraView> {
    let view_dir = PathBuf::from(engines::DEFAULT_ASSET_FOLDER).join("views");
    engines::TeraView::from_custom_dir(&view_dir, configure_tera)
}

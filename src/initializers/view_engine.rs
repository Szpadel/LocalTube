use async_trait::async_trait;
use axum::{Extension, Router as AxumRouter};
use fluent_templates::{ArcLoader, FluentLoader};
use loco_rs::{
    app::{AppContext, Initializer},
    controller::views::{engines, tera_builtins, ViewEngine},
    environment::Environment,
    Error, Result,
};
use std::path::{Path, PathBuf};
#[cfg(debug_assertions)]
use std::sync::{Arc, Mutex};
use tracing::info;

const I18N_DIR: &str = "assets/i18n";
const I18N_SHARED: &str = "assets/i18n/shared.ftl";

pub struct ViewEngineInitializer;
#[async_trait]
impl Initializer for ViewEngineInitializer {
    fn name(&self) -> String {
        "view-engine".to_string()
    }

    async fn after_routes(&self, router: AxumRouter, ctx: &AppContext) -> Result<AxumRouter> {
        let tera_engine = match ctx.environment {
            Environment::Test => build_test_tera_engine()?,
            _ => engines::TeraView::build()?,
        }
        .post_process(|tera| {
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
        })?;

        Ok(router.layer(Extension(ViewEngine::from(tera_engine))))
    }
}

/// Builds a Tera view engine without file watching for test environments.
///
/// # Errors
///
/// Returns an error if view templates cannot be loaded.
pub fn build_test_tera_engine() -> Result<engines::TeraView> {
    let view_path = PathBuf::from(engines::DEFAULT_ASSET_FOLDER)
        .join("views")
        .join("**")
        .join("*.html");
    let view_glob = view_path
        .to_str()
        .ok_or_else(|| Error::string("invalid glob"))?;
    let mut engine = tera::Tera::new(view_glob)?;
    tera_builtins::filters::register_filters(&mut engine);

    #[cfg(debug_assertions)]
    let tera = {
        let engine = engines::HotReloadingTeraEngine {
            engine,
            view_path,
            file_watcher: None,
            dirty: false,
        };
        Arc::new(Mutex::new(engine))
    };

    #[cfg(not(debug_assertions))]
    let tera = engine;

    Ok(engines::TeraView {
        tera,
        tera_post_process: None,
        default_context: tera::Context::default(),
    })
}

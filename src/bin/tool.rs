use localtube::app::App;
use loco_rs::cli;
use migration::Migrator;

#[tokio::main]
#[allow(clippy::result_large_err)]
async fn main() -> loco_rs::Result<()> {
    cli::main::<App, Migrator>().await
}

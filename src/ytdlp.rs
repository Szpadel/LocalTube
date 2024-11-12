use loco_rs::{Error, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::OnceLock;
use tokio::sync::Semaphore;
use tokio::{io::AsyncBufReadExt, process::Command};
use tokio_process_terminate::TerminateExt;
use tracing::{info, warn};
use yt_dlp::fetcher::deps::Libraries;

const LIBS_DIR: &str = "libs";
static CONCURRENCY_SEMAPHORE: OnceLock<Semaphore> = OnceLock::new();

fn ytdtp_concurrency() -> &'static Semaphore {
    const ENV_CONCURRENCY: &str = "LOCALTUBE_YTDLP_CONCURRENCY";
    CONCURRENCY_SEMAPHORE.get_or_init(|| {
        let concurrency = std::env::var(ENV_CONCURRENCY)
            .ok()
            .and_then(|v| {
                v.parse::<usize>()
                    .map_err(|e| {
                        warn!(
                            "Warning: {} value '{}' is invalid: {}",
                            ENV_CONCURRENCY, v, e
                        );
                    })
                    .ok()
            })
            .unwrap_or(4);

        let limited_concurrency = concurrency.clamp(1, 8);
        if limited_concurrency != concurrency {
            warn!(
                "Warning: {} value {} is outside allowed range (1-8), using {}",
                ENV_CONCURRENCY, concurrency, limited_concurrency
            );
        }

        info!("yt-dlp concurrency: {}", limited_concurrency);

        Semaphore::new(limited_concurrency)
    })
}

static MEDIA_DIRECTORY: OnceLock<PathBuf> = OnceLock::new();

/// Returns the configured media directory path
#[must_use]
pub fn media_directory() -> &'static PathBuf {
    MEDIA_DIRECTORY.get_or_init(|| {
        std::env::var("LOCALTUBE_MEDIA_DIR").map_or_else(
            |_| {
                warn!("Warning: LOCALTUBE_MEDIA_DIR not set, using default: media");
                PathBuf::from("media")
            },
            PathBuf::from,
        )
    })
}

/// Returns the path to the yt-dlp executable
#[must_use]
pub fn yt_dlp_path() -> PathBuf {
    PathBuf::from(LIBS_DIR).join("yt-dlp")
}

/// Returns the path to the ffmpeg executable
#[must_use]
pub fn ffmpeg_path() -> PathBuf {
    PathBuf::from(LIBS_DIR).join("ffmpeg")
}

/// Downloads required dependencies
///
/// # Errors
///
/// Returns error if download or installation fails
pub async fn download_deps() -> Result<(), yt_dlp::error::Error> {
    let yt_dlp = yt_dlp_path();
    let ffmpeg = ffmpeg_path();
    let libraries = Libraries::new(yt_dlp, ffmpeg);
    libraries.install_dependencies().await?;
    Ok(())
}

#[derive(Deserialize, Serialize)]
pub struct VideoMetadata {
    pub title: String,
    pub description: Option<String>,
    pub duration: u64,
    pub uploader: String,
    pub n_entries: Option<u64>,
    pub extractor_key: String,
    pub original_url: String,
    pub timestamp: i64,
    pub filename: String,
}

/// Downloads metadata for the last video from given URL
///
/// # Panics
///
/// Panics if unable to acquire concurrency semaphore
///
/// # Errors
///
/// Returns error if download fails or response parsing fails
pub async fn download_last_video_metadata(url: &str) -> Result<VideoMetadata> {
    let _s = ytdtp_concurrency().acquire().await.unwrap();
    let output = Command::new(yt_dlp_path())
        .arg("--dump-json")
        .arg("--max-downloads=1")
        .arg("--simulate")
        .arg(url)
        .output()
        .await?;

    let video_metadata: VideoMetadata = serde_json::from_slice(&output.stdout)?;
    Ok(video_metadata)
}

pub async fn stream_media_list(url: &str) -> tokio::sync::mpsc::Receiver<VideoMetadata> {
    let (tx, rx) = tokio::sync::mpsc::channel(8);
    let s = ytdtp_concurrency().acquire().await.unwrap();
    let url = url.to_string();
    tokio::spawn(async move {
        let _s = s;
        let mut cmd = Command::new(yt_dlp_path())
            .process_group(0)
            .arg("--dump-json")
            .arg("--simulate")
            .arg(url)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .expect("Failed to spawn yt-dlp");

        let output = cmd.stdout.take().expect("Failed to get yt-dlp stdout");
        let reader = tokio::io::BufReader::new(output);
        let mut lines = tokio::io::BufReader::new(reader).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if line.is_empty() {
                continue;
            }
            let video_metadata: VideoMetadata = serde_json::from_str(&line).unwrap();
            if tx.send(video_metadata).await.is_err() || tx.is_closed() {
                // Receiver was dropped, terminate the command
                cmd.terminate_wait().await.unwrap();
                break;
            }
        }
    });
    rx
}

/// Downloads media from given URL
///
/// # Panics
///
/// Panics if unable to acquire concurrency semaphore
///
/// # Errors
///
/// Returns error if download fails, source metadata is missing or invalid paths are encountered
pub async fn download_media(
    url: &str,
    source: &crate::models::_entities::sources::Model,
) -> Result<String> {
    let _s = ytdtp_concurrency().acquire().await.unwrap();
    let media_dir = media_directory();
    let source_name = source
        .get_metadata()
        .map(|m| {
            m.uploader
                .chars()
                .filter(|c| {
                    c.is_alphanumeric()
                        || matches!(c, '-' | '_' | ' ' | '.' | '(' | ')' | '[' | ']')
                })
                .collect::<String>()
        })
        .ok_or_else(|| Error::string("Missing source metadata"))?;
    let source_dir = media_dir.join(source_name);
    tokio::fs::create_dir_all(&source_dir).await?;
    let output = Command::new(yt_dlp_path())
        .arg("--dump-json")
        .arg("--restrict-filenames")
        .arg("--write-info-json")
        .arg(format!("--paths={}", source_dir.display()))
        .arg("--max-downloads=1")
        .arg("--no-simulate")
        .arg(url)
        .output()
        .await?;

    let video_metadata: VideoMetadata = serde_json::from_slice(&output.stdout)?;
    Ok(PathBuf::from(&video_metadata.filename)
        .strip_prefix(media_dir)
        .map_err(|_| Error::string("Invalid media path"))?
        .to_string_lossy()
        .to_string())
}

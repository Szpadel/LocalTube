use crate::ytdlp_debug;
use loco_rs::{Error, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use tokio::sync::Semaphore;
use tokio::{io::AsyncBufReadExt, process::Command};
use tokio_process_terminate::TerminateExt;
use tracing::{info, warn};
use yt_dlp::fetcher::deps::Libraries;

const LIBS_DIR: &str = "libs";
const STREAM_ERROR_MESSAGE: &str = "yt-dlp stream failed; check logs for details";
static CONCURRENCY_SEMAPHORE: OnceLock<Arc<Semaphore>> = OnceLock::new();

pub fn ytdtp_concurrency() -> &'static Arc<Semaphore> {
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

        Arc::new(Semaphore::new(limited_concurrency))
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
/// # Errors
///
/// Returns error if download fails or response parsing fails
///
/// # Note
///
/// This function does not acquire the concurrency semaphore. The caller
/// must ensure proper concurrency control (typically via `ActiveTask`).
pub async fn download_last_video_metadata(url: &str) -> Result<VideoMetadata> {
    let output = Command::new(yt_dlp_path())
        .arg("--dump-json")
        .arg("-t")
        .arg("sleep")
        .arg("--max-downloads=1")
        .arg("--simulate")
        .arg(url)
        .output()
        .await?;

    ytdlp_debug::log_ytdlp_json(
        "download_last_video_metadata",
        &output.stdout,
        Some(url),
        None,
    )
    .await;
    let video_metadata: VideoMetadata = serde_json::from_slice(&output.stdout)?;
    Ok(video_metadata)
}

/// Streams media list from a given URL
///
/// # Panics
///
/// Panics if spawning the `yt-dlp` process or capturing its output fails.
///
/// # Note
///
/// The returned stream yields `Ok(VideoMetadata)` entries. If the process
/// exits non-zero or produces zero items, a single `Err` is sent before
/// closing the channel.
///
/// # Note
///
/// This function does not acquire the concurrency semaphore. The caller
/// must ensure proper concurrency control (typically via `ActiveTask`).
pub async fn stream_media_list(url: &str) -> tokio::sync::mpsc::Receiver<Result<VideoMetadata>> {
    let (tx, rx) = tokio::sync::mpsc::channel(8);
    let url = url.to_string();
    tokio::spawn(async move {
        let mut cmd = Command::new(yt_dlp_path())
            .process_group(0)
            .arg("--dump-json")
            .arg("--simulate")
            .arg("-t")
            .arg("sleep")
            .arg(&url)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .expect("Failed to spawn yt-dlp");

        let stdout = cmd.stdout.take().expect("Failed to get yt-dlp stdout");
        let stderr = cmd.stderr.take().expect("Failed to get yt-dlp stderr");
        let mut stdout_lines = tokio::io::BufReader::new(stdout).lines();
        let mut stderr_lines = tokio::io::BufReader::new(stderr).lines();
        let mut stdout_done = false;
        let mut stderr_done = false;
        let mut items_emitted = 0usize;

        while !stdout_done || !stderr_done {
            tokio::select! {
                line = stdout_lines.next_line(), if !stdout_done => {
                    match line {
                        Ok(Some(line)) => {
                            if line.is_empty() {
                                continue;
                            }
                            ytdlp_debug::log_ytdlp_line("stream_media_list", &line, Some(&url), None).await;
                            let video_metadata = match serde_json::from_str::<VideoMetadata>(&line) {
                                Ok(metadata) => metadata,
                                Err(err) => {
                                    warn!(error = %err, "failed to parse yt-dlp JSON line");
                                    continue;
                                }
                            };
                            if tx.send(Ok(video_metadata)).await.is_err() || tx.is_closed() {
                                // Receiver was dropped, terminate the command
                                if let Err(err) = cmd.terminate_wait().await {
                                    warn!(error = %err, "failed to terminate yt-dlp");
                                }
                                return;
                            }
                            items_emitted += 1;
                        }
                        Ok(None) => {
                            stdout_done = true;
                        }
                        Err(err) => {
                            warn!(error = %err, "failed to read yt-dlp stdout line");
                            stdout_done = true;
                        }
                    }
                }
                line = stderr_lines.next_line(), if !stderr_done => {
                    match line {
                        Ok(Some(line)) => {
                            if line.is_empty() {
                                continue;
                            }
                            ytdlp_debug::log_ytdlp_line("stream_media_list_stderr", &line, Some(&url), None).await;
                        }
                        Ok(None) => {
                            stderr_done = true;
                        }
                        Err(err) => {
                            warn!(error = %err, "failed to read yt-dlp stderr line");
                            stderr_done = true;
                        }
                    }
                }
            }
        }

        let exit_success = match cmd.wait().await {
            Ok(status) => status.success(),
            Err(err) => {
                warn!(error = %err, "failed to wait on yt-dlp");
                false
            }
        };

        if stream_should_fail(exit_success, items_emitted) {
            let _ = tx.send(Err(Error::string(STREAM_ERROR_MESSAGE))).await;
        }
    });
    rx
}

fn stream_should_fail(exit_success: bool, items_emitted: usize) -> bool {
    !exit_success || items_emitted == 0
}

/// Downloads media from given URL
///
/// # Errors
///
/// Returns error if download fails, source metadata is missing or invalid paths are encountered
///
/// # Note
///
/// This function does not acquire the concurrency semaphore. The caller
/// must ensure proper concurrency control (typically via `ActiveTask`).
pub async fn download_media(
    url: &str,
    source: &crate::models::_entities::sources::Model,
) -> Result<String> {
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
    // we reserialize to ensure we have only valid input
    let sponsorblock = source.get_sponsorblock_categories().serialize();
    let output = Command::new(yt_dlp_path())
        .arg("--dump-json")
        .arg("-t")
        .arg("sleep")
        .arg("--restrict-filenames")
        .arg("--write-info-json")
        .arg(format!(
            "--sponsorblock-remove={}",
            if sponsorblock.is_empty() {
                "-all"
            } else {
                &sponsorblock
            }
        ))
        .arg(format!("--paths={}", source_dir.display()))
        .arg("--max-downloads=1")
        .arg("--no-simulate")
        .arg("--remux-video=mkv")
        .arg("--embed-metadata")
        .arg("--embed-subs")
        .arg("--embed-thumbnail")
        .arg(url)
        .output()
        .await?;

    ytdlp_debug::log_ytdlp_json(
        "download_media",
        &output.stdout,
        Some(url),
        Some(&format!("source_id={}", source.id)),
    )
    .await;
    let video_metadata: VideoMetadata = serde_json::from_slice(&output.stdout)?;

    // yt-dlp do not report remuxed file path, we need to check if it exists
    // check if video_metadata.filename with .mkv extension exists if not check if video_metadata.filename exists
    // use existing file if it exists, error out if none exists
    let video_path = PathBuf::from(&video_metadata.filename);
    let video_path = if video_path.with_extension("mkv").exists() {
        video_path.with_extension("mkv")
    } else if video_path.exists() {
        video_path
    } else {
        return Err(Error::string("Failed to download media"));
    };

    Ok(PathBuf::from(&video_path)
        .strip_prefix(media_dir)
        .map_err(|_| Error::string("Invalid media path"))?
        .to_string_lossy()
        .to_string())
}

#[cfg(test)]
mod tests {
    use super::stream_should_fail;

    #[test]
    fn stream_should_fail_when_exit_success_but_no_items() {
        assert!(stream_should_fail(true, 0));
    }

    #[test]
    fn stream_should_fail_when_exit_failure_even_with_items() {
        assert!(stream_should_fail(false, 3));
    }

    #[test]
    fn stream_should_succeed_when_exit_success_and_items_present() {
        assert!(!stream_should_fail(true, 2));
    }
}

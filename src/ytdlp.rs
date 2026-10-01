use crate::ytdlp_debug;
use loco_rs::{Error, Result};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Output;
use std::sync::{Arc, OnceLock};
use tokio::sync::Semaphore;
use tokio::{io::AsyncBufReadExt, process::Command};
use tokio_process_terminate::TerminateExt;
use tracing::{info, warn};
use yt_dlp::client::deps::Libraries;

const LIBS_DIR: &str = "libs";
const STREAM_ERROR_MESSAGE: &str = "yt-dlp stream failed; check logs for details";
/// yt-dlp starts each error report on stderr with this prefix.
const ERROR_LINE_PREFIX: &str = "ERROR:";
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

/// Updates the yt-dlp binary to the latest stable release.
///
/// `yt-dlp --update` replaces its own file in steps: it renames the old file
/// away, renames the new file into place, and then makes it executable. A
/// yt-dlp start between these steps fails. Thus this function updates a copy
/// and renames the copy over `libs/yt-dlp` in one atomic step. yt-dlp
/// processes that run at that time keep the old file open and continue to
/// operate.
///
/// A failed attempt can leave the copy at `libs/yt-dlp.staged`. The next
/// attempt replaces it.
///
/// Returns the last line that yt-dlp wrote, for example
/// `Updated yt-dlp to stable@2026.08.19 from yt-dlp/yt-dlp`.
///
/// # Errors
///
/// Returns an error if the copy, the update, or the rename fails. In each
/// case, `libs/yt-dlp` stays unchanged.
pub async fn update_yt_dlp() -> Result<String> {
    update_binary_atomically(&yt_dlp_path()).await
}

/// Runs `--update` on a staged copy of the yt-dlp binary at `installed` and
/// renames the copy over `installed`. See [`update_yt_dlp`].
async fn update_binary_atomically(installed: &Path) -> Result<String> {
    let staged = installed.with_extension("staged");
    tokio::fs::copy(installed, &staged).await.map_err(|error| {
        Error::string(&format!(
            "Failed to copy {} to {}: {error}",
            installed.display(),
            staged.display()
        ))
    })?;

    // When shutdown cancels this future, the update must stop too. Otherwise
    // it continues to change the staged copy after the app stops.
    let output = Command::new(&staged)
        .arg("--update")
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|error| {
            Error::string(&format!("Failed to start {}: {error}", staged.display()))
        })?;
    if !output.status.success() {
        return Err(Error::string(&format!(
            "yt-dlp update failed {}",
            describe_run(&output)
        )));
    }

    tokio::fs::rename(&staged, installed)
        .await
        .map_err(|error| {
            Error::string(&format!(
                "Failed to rename {} to {}: {error}",
                staged.display(),
                installed.display()
            ))
        })?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout.lines().last().unwrap_or_default().to_string())
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

#[derive(Debug, PartialEq, Eq, Clone, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceListKind {
    Video,
    List,
}

#[derive(Debug, PartialEq, Eq, Clone, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceListOrder {
    NewestFirst,
    OldestFirst,
}

#[derive(Debug, PartialEq, Eq, Clone, Deserialize, Serialize)]
pub struct SourceListTabOption {
    pub url: String,
    pub label: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaListOrder {
    Original,
    Reverse,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListProbeMode {
    Minimal,
    OrderAware,
}

#[derive(Debug, Clone)]
pub struct ListProbe {
    pub list_kind: SourceListKind,
    pub list_count: Option<u64>,
    pub list_order: Option<SourceListOrder>,
    pub uploader: Option<String>,
    pub source_provider: Option<String>,
}

#[derive(Deserialize)]
struct ProbeOutput {
    #[serde(rename = "_type")]
    kind: Option<String>,
    playlist_count: Option<u64>,
    uploader: Option<String>,
    extractor_key: Option<String>,
    entries: Option<Vec<Option<ProbeEntry>>>,
}

#[derive(Deserialize)]
struct ProbeEntry {
    #[serde(rename = "_type")]
    kind: Option<String>,
    timestamp: Option<i64>,
    upload_date: Option<String>,
    uploader: Option<String>,
    extractor_key: Option<String>,
    playlist_count: Option<u64>,
    webpage_url: Option<String>,
    url: Option<String>,
    title: Option<String>,
}

fn flatten_probe_entries(entries: Option<Vec<Option<ProbeEntry>>>) -> Vec<ProbeEntry> {
    entries.unwrap_or_default().into_iter().flatten().collect()
}

/// Describes how a finished yt-dlp run ended: its exit status and the `ERROR:` lines from stderr.
///
/// Example: `(exit status: 1) ERROR: [youtube] abc: Video unavailable`.
fn describe_run(output: &Output) -> String {
    let mut description = format!("({})", output.status);
    let stderr = String::from_utf8_lossy(&output.stderr);
    for error_line in stderr
        .lines()
        .filter(|line| line.starts_with(ERROR_LINE_PREFIX))
    {
        description.push(' ');
        description.push_str(error_line);
    }
    description
}

/// Parses the JSON document that yt-dlp wrote to stdout.
///
/// # Errors
///
/// Returns an error with [`describe_run`] when stdout is empty or is not valid JSON for `T`.
/// When extraction fails, yt-dlp writes nothing to stdout, and only its `ERROR:` lines give the cause.
fn parse_stdout_json<T: DeserializeOwned>(output: &Output) -> Result<T> {
    if output.stdout.trim_ascii().is_empty() {
        return Err(Error::string(&format!(
            "yt-dlp wrote no JSON {}",
            describe_run(output)
        )));
    }
    serde_json::from_slice(&output.stdout).map_err(|parse_error| {
        Error::string(&format!(
            "yt-dlp wrote invalid JSON: {parse_error} {}",
            describe_run(output)
        ))
    })
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
    let video_metadata: VideoMetadata = parse_stdout_json(&output)?;
    Ok(video_metadata)
}

/// Probes list metadata for the given URL.
///
/// # Errors
///
/// Returns error if yt-dlp fails or the response parsing fails.
pub async fn probe_list_metadata(url: &str, mode: ListProbeMode) -> Result<ListProbe> {
    let item_spec = match mode {
        ListProbeMode::Minimal => "1:1",
        ListProbeMode::OrderAware => "1:2",
    };
    let output = Command::new(yt_dlp_path())
        .arg("--dump-single-json")
        .arg("-I")
        .arg(item_spec)
        .arg("--simulate")
        .arg(url)
        .output()
        .await?;

    // Use a tiny probe to avoid loading entire large lists just to detect order/count.
    ytdlp_debug::log_ytdlp_json("probe_list_metadata", &output.stdout, Some(url), None).await;
    let probe: ProbeOutput = parse_stdout_json(&output)?;
    let ProbeOutput {
        kind,
        playlist_count,
        uploader: probe_uploader,
        extractor_key: probe_extractor_key,
        entries,
    } = probe;
    let entries = flatten_probe_entries(entries);
    let list_kind = match kind.as_deref() {
        Some("video") => SourceListKind::Video,
        Some("playlist") => SourceListKind::List,
        _ => {
            if entries.is_empty() {
                SourceListKind::Video
            } else {
                SourceListKind::List
            }
        }
    };

    let list_count = playlist_count.or_else(|| entries.first().and_then(|e| e.playlist_count));

    let uploader = entries
        .first()
        .and_then(|e| e.uploader.clone())
        .or(probe_uploader);
    let source_provider = entries
        .first()
        .and_then(|e| e.extractor_key.clone())
        .or(probe_extractor_key);

    let list_order = match mode {
        ListProbeMode::OrderAware => detect_list_order(&entries),
        ListProbeMode::Minimal => None,
    };

    Ok(ListProbe {
        list_kind,
        list_count,
        list_order,
        uploader,
        source_provider,
    })
}

/// Probes list tabs for the given URL using a flat, tiny request.
///
/// # Errors
///
/// Returns error if yt-dlp fails or the response parsing fails.
pub async fn probe_list_tabs(url: &str) -> Result<Vec<SourceListTabOption>> {
    const TAB_PROBE_MAX: usize = 10;
    let run_probe = |flat: bool| async move {
        let mut cmd = Command::new(yt_dlp_path());
        cmd.arg("--dump-single-json")
            .arg("-I")
            // Use a small cap to avoid scanning huge lists while still capturing all tabs.
            .arg(format!("1:{TAB_PROBE_MAX}"))
            .arg("--simulate")
            .arg(url);
        if flat {
            cmd.arg("--flat-playlist");
        }
        let output = cmd.output().await?;
        let extra = if flat { Some("flat") } else { None };
        // Small capped probe prevents expanding the entire channel while still exposing tab URLs.
        ytdlp_debug::log_ytdlp_json("probe_list_tabs", &output.stdout, Some(url), extra).await;
        let probe: ProbeOutput = parse_stdout_json(&output)?;
        let entries = flatten_probe_entries(probe.entries);
        Ok::<_, Error>(extract_list_tabs(&entries))
    };

    let mut tabs = run_probe(false).await?;
    if tabs.is_empty() {
        // Some extractors only expose tab URLs in flat playlist mode.
        tabs = run_probe(true).await?;
    }
    Ok(tabs)
}

fn extract_list_tabs(entries: &[ProbeEntry]) -> Vec<SourceListTabOption> {
    let mut tabs = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for entry in entries {
        let url = entry
            .webpage_url
            .as_ref()
            .or(entry.url.as_ref())
            .map(|u| normalize_tab_url(u));
        let Some(url) = url else {
            continue;
        };
        if !entry_is_tab_candidate(entry, &url) {
            continue;
        }
        if !seen.insert(url.clone()) {
            continue;
        }
        let label = known_tab_label(&url)
            .map(str::to_string)
            .or_else(|| entry.title.clone())
            .unwrap_or_else(|| url.clone());
        tabs.push(SourceListTabOption { url, label });
    }
    tabs
}

fn entry_is_tab_candidate(entry: &ProbeEntry, url: &str) -> bool {
    let known_tab = known_tab_label(url).is_some();
    if known_tab {
        return true;
    }
    if matches!(entry.kind.as_deref(), Some("playlist")) {
        return matches!(entry.extractor_key.as_deref(), Some("YoutubeTab"));
    }
    matches!(entry.kind.as_deref(), Some("url") | None) && known_tab
}

fn normalize_tab_url(url: &str) -> String {
    url.trim_end_matches('/').to_string()
}

fn known_tab_label(url: &str) -> Option<&'static str> {
    let url = url.split(['?', '#']).next().unwrap_or(url);
    let url = url.trim_end_matches('/');
    if url.ends_with("/videos") {
        Some("Videos")
    } else if url.ends_with("/streams") {
        Some("Streams")
    } else if url.ends_with("/shorts") {
        Some("Shorts")
    } else if url.ends_with("/playlists") {
        Some("Playlists")
    } else {
        None
    }
}

fn detect_list_order(entries: &[ProbeEntry]) -> Option<SourceListOrder> {
    if entries.len() < 2 {
        return None;
    }

    let first = entry_timestamp(&entries[0])?;
    let second = entry_timestamp(&entries[1])?;
    if first == second {
        return None;
    }

    if first < second {
        Some(SourceListOrder::OldestFirst)
    } else {
        Some(SourceListOrder::NewestFirst)
    }
}

fn entry_timestamp(entry: &ProbeEntry) -> Option<i64> {
    entry
        .timestamp
        .or_else(|| entry.upload_date.as_deref().and_then(parse_upload_date))
}

fn parse_upload_date(value: &str) -> Option<i64> {
    if value.len() != 8 || !value.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    value.parse::<i64>().ok()
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
pub async fn stream_media_list(
    url: &str,
    order: MediaListOrder,
) -> tokio::sync::mpsc::Receiver<Result<VideoMetadata>> {
    let (tx, rx) = tokio::sync::mpsc::channel(8);
    let url = url.to_string();
    tokio::spawn(async move {
        let mut cmd = Command::new(yt_dlp_path())
            .process_group(0)
            .arg("--dump-json")
            .arg("--simulate")
            .arg("-t")
            .arg("sleep")
            .args(match order {
                MediaListOrder::Original => &[][..],
                MediaListOrder::Reverse => &["-I", "::-1"][..],
            })
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
    let video_metadata: VideoMetadata = parse_stdout_json(&output)?;

    // yt-dlp do not report remuxed file path, we need to check if it exists
    // check if video_metadata.filename with .mkv extension exists if not check if video_metadata.filename exists
    // use existing file if it exists, error out if none exists
    let video_path = PathBuf::from(&video_metadata.filename);
    let video_path = if video_path.with_extension("mkv").exists() {
        video_path.with_extension("mkv")
    } else if video_path.exists() {
        video_path
    } else {
        return Err(Error::string(&format!(
            "yt-dlp created no media file {}",
            describe_run(&output)
        )));
    };

    Ok(PathBuf::from(&video_path)
        .strip_prefix(media_dir)
        .map_err(|_| Error::string("Invalid media path"))?
        .to_string_lossy()
        .to_string())
}

#[cfg(test)]
mod tests {
    use super::{
        detect_list_order, extract_list_tabs, flatten_probe_entries, parse_stdout_json,
        stream_should_fail, update_binary_atomically, ProbeEntry, ProbeOutput, SourceListOrder,
        SourceListTabOption,
    };
    use serial_test::serial;
    use std::os::unix::{fs::PermissionsExt, process::ExitStatusExt};
    use std::path::{Path, PathBuf};
    use std::process::{ExitStatus, Output};

    /// Temporary directory that is removed when the value is dropped.
    struct ScratchDir(PathBuf);

    impl ScratchDir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("localtube-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&path).expect("scratch directory should be created");
            Self(path)
        }
    }

    impl Drop for ScratchDir {
        fn drop(&mut self) {
            // A leftover directory in the system temp directory does not affect other tests.
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn write_script(path: &Path, body: &str) {
        std::fs::write(path, format!("#!/bin/sh\n{body}\n")).expect("script should be written");
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
            .expect("script should be made executable");
    }

    // Serial: a process start in a parallel test can inherit the write handle
    // of a new script for a moment, and then the start of that script fails.
    #[tokio::test]
    #[serial]
    async fn failed_update_keeps_installed_binary() {
        let scratch = ScratchDir::new();
        let installed = scratch.0.join("yt-dlp");
        write_script(
            &installed,
            "echo 'ERROR: Unable to obtain version info' >&2\nexit 100",
        );
        let installed_before = std::fs::read(&installed).expect("binary should be readable");

        let error = update_binary_atomically(&installed)
            .await
            .expect_err("update must fail")
            .to_string();

        assert_eq!(
            error,
            "yt-dlp update failed (exit status: 100) ERROR: Unable to obtain version info"
        );
        assert_eq!(
            std::fs::read(&installed).expect("binary should be readable"),
            installed_before
        );
    }

    #[tokio::test]
    #[serial]
    async fn successful_update_replaces_installed_binary() {
        let scratch = ScratchDir::new();
        let installed = scratch.0.join("yt-dlp");
        // Like `yt-dlp --update`, the script replaces its own file with the new release.
        write_script(
            &installed,
            "printf '#!/bin/sh\\necho 2026.08.19\\n' > \"$0.new\"\n\
             chmod 755 \"$0.new\"\n\
             mv \"$0.new\" \"$0\"\n\
             echo 'Updated yt-dlp to stable@2026.08.19 from yt-dlp/yt-dlp'",
        );

        let result = update_binary_atomically(&installed)
            .await
            .expect("update must succeed");
        let version = tokio::process::Command::new(&installed)
            .output()
            .await
            .expect("updated binary should run");

        assert_eq!(
            result,
            "Updated yt-dlp to stable@2026.08.19 from yt-dlp/yt-dlp"
        );
        assert_eq!(String::from_utf8_lossy(&version.stdout), "2026.08.19\n");
        assert!(!installed.with_extension("staged").exists());
    }

    fn finished_run(exit_code: i32, stdout: &str, stderr: &str) -> Output {
        Output {
            // A wait status stores the exit code in its second byte.
            status: ExitStatus::from_raw(exit_code << 8),
            stdout: stdout.as_bytes().to_vec(),
            stderr: stderr.as_bytes().to_vec(),
        }
    }

    #[test]
    fn parse_stdout_json_reports_yt_dlp_error_when_stdout_is_empty() {
        let run = finished_run(
            1,
            "",
            "WARNING: [youtube] SsYKLXIU7no: Some web client https formats have been skipped\n\
             ERROR: [youtube] SsYKLXIU7no: The page needs to be reloaded.\n",
        );

        let error = parse_stdout_json::<serde_json::Value>(&run)
            .expect_err("empty stdout must fail")
            .to_string();

        assert_eq!(
            error,
            "yt-dlp wrote no JSON (exit status: 1) \
             ERROR: [youtube] SsYKLXIU7no: The page needs to be reloaded."
        );
    }

    #[test]
    fn parse_stdout_json_reports_parse_error_for_invalid_json() {
        let run = finished_run(0, "{\"title\":", "");

        let error = parse_stdout_json::<serde_json::Value>(&run)
            .expect_err("truncated JSON must fail")
            .to_string();

        assert!(
            error.starts_with("yt-dlp wrote invalid JSON: EOF while parsing"),
            "{error}"
        );
        assert!(error.ends_with(" (exit status: 0)"), "{error}");
    }

    #[test]
    fn parse_stdout_json_accepts_json_from_run_with_max_downloads_reached() {
        // yt-dlp exits with status 101 after it reaches `--max-downloads`.
        let run = finished_run(101, "{\"title\":\"Video\"}\n", "");

        let value = parse_stdout_json::<serde_json::Value>(&run).expect("valid JSON must parse");

        assert_eq!(value["title"], "Video");
    }

    fn entry(timestamp: Option<i64>, upload_date: Option<&str>) -> ProbeEntry {
        ProbeEntry {
            kind: None,
            timestamp,
            upload_date: upload_date.map(str::to_string),
            uploader: None,
            extractor_key: None,
            playlist_count: None,
            webpage_url: None,
            url: None,
            title: None,
        }
    }

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

    #[test]
    fn detect_list_order_uses_timestamps() {
        let entries = vec![entry(Some(100), None), entry(Some(200), None)];
        assert_eq!(
            detect_list_order(&entries),
            Some(SourceListOrder::OldestFirst)
        );
    }

    #[test]
    fn detect_list_order_uses_upload_date_fallback() {
        let entries = vec![entry(None, Some("20240101")), entry(None, Some("20240102"))];
        assert_eq!(
            detect_list_order(&entries),
            Some(SourceListOrder::OldestFirst)
        );
    }

    #[test]
    fn detect_list_order_returns_none_when_undetermined() {
        let entries = vec![entry(None, None), entry(None, None)];
        assert_eq!(detect_list_order(&entries), None);
    }

    #[test]
    fn extract_list_tabs_filters_non_tab_urls() {
        let entries = vec![
            ProbeEntry {
                kind: Some("url".to_string()),
                timestamp: None,
                upload_date: None,
                uploader: None,
                extractor_key: None,
                playlist_count: None,
                webpage_url: Some("https://example.com/other".to_string()),
                url: None,
                title: Some("Other".to_string()),
            },
            ProbeEntry {
                kind: Some("url".to_string()),
                timestamp: None,
                upload_date: None,
                uploader: None,
                extractor_key: None,
                playlist_count: None,
                webpage_url: Some("https://example.com/videos".to_string()),
                url: None,
                title: Some("Videos".to_string()),
            },
        ];
        assert_eq!(
            extract_list_tabs(&entries),
            vec![SourceListTabOption {
                url: "https://example.com/videos".to_string(),
                label: "Videos".to_string(),
            }]
        );
    }

    #[test]
    fn probe_output_skips_null_entries() {
        let json = r#"{"_type":"playlist","entries":[null,{"_type":"url","webpage_url":"https://example.com/videos","title":"Videos"}]}"#;
        let probe: ProbeOutput = serde_json::from_str(json).expect("probe json");
        let entries = flatten_probe_entries(probe.entries);
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].webpage_url.as_deref(),
            Some("https://example.com/videos")
        );
    }
}

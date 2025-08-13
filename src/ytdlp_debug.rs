use std::path::PathBuf;
use std::sync::OnceLock;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;
use tracing::debug;

enum Mode {
    Off,
    Log,
    File(PathBuf),
}

struct Cfg {
    mode: Mode,
}

static CFG: OnceLock<Cfg> = OnceLock::new();
static FILE_LOCK: Mutex<()> = Mutex::const_new(());

fn cfg() -> &'static Cfg {
    CFG.get_or_init(|| {
        let v = std::env::var("LOCALTUBE_YTDLP_DEBUG").unwrap_or_else(|_| "off".to_string());
        let v = v.trim();
        let mode = if v.eq_ignore_ascii_case("off") || v.is_empty() {
            Mode::Off
        } else if v.eq_ignore_ascii_case("log") {
            Mode::Log
        } else if let Some(path) = v.strip_prefix("file:") {
            let p = if path.is_empty() {
                PathBuf::from("logs/ytdlp-json.log")
            } else {
                PathBuf::from(path)
            };
            Mode::File(p)
        } else if v.eq_ignore_ascii_case("file") {
            Mode::File(PathBuf::from("logs/ytdlp-json.log"))
        } else {
            Mode::Off
        };
        Cfg { mode }
    })
}

fn compact_json(bytes: &[u8]) -> Option<String> {
    serde_json::from_slice::<serde_json::Value>(bytes)
        .ok()
        .map(|v| serde_json::to_string(&v).unwrap_or_default())
}

fn sanitize_line(s: &str) -> String {
    s.replace(['\n', '\r'], " ")
}

fn prefix(ctx: &str, url: Option<&str>, len: usize, extra: Option<&str>) -> String {
    let mut p = String::from("[ytdlp");
    p.push_str(" ctx=");
    p.push_str(ctx);
    if let Some(u) = url {
        p.push_str(" url=");
        p.push_str(u);
    }
    p.push_str(" len=");
    p.push_str(len.to_string().as_str());
    if let Some(e) = extra {
        p.push(' ');
        p.push_str(e);
    }
    p.push_str("] ");
    p
}

pub async fn log_ytdlp_json(ctx: &str, bytes: &[u8], url: Option<&str>, extra: Option<&str>) {
    match &cfg().mode {
        Mode::Off => {}
        Mode::Log => {
            let payload = compact_json(bytes)
                .unwrap_or_else(|| sanitize_line(&String::from_utf8_lossy(bytes)));
            let line = format!("{}{}", prefix(ctx, url, bytes.len(), extra), payload);
            debug!(target = "ytdlp_json", "{}", line);
        }
        Mode::File(path) => {
            let payload = compact_json(bytes)
                .unwrap_or_else(|| sanitize_line(&String::from_utf8_lossy(bytes)));
            let line = format!("{}{}\n", prefix(ctx, url, bytes.len(), extra), payload);
            let _g = FILE_LOCK.lock().await;
            if let Some(parent) = path.parent() {
                let _ = tokio::fs::create_dir_all(parent).await;
            }
            if let Ok(mut f) = tokio::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .await
            {
                let _ = f.write_all(line.as_bytes()).await;
                let _ = f.flush().await;
            }
        }
    }
}

pub async fn log_ytdlp_line(ctx: &str, line: &str, url: Option<&str>, extra: Option<&str>) {
    match &cfg().mode {
        Mode::Off => {}
        Mode::Log => {
            let payload = compact_json(line.as_bytes()).unwrap_or_else(|| sanitize_line(line));
            let out = format!("{}{}", prefix(ctx, url, line.len(), extra), payload);
            debug!(target = "ytdlp_json", "{}", out);
        }
        Mode::File(path) => {
            let payload = compact_json(line.as_bytes()).unwrap_or_else(|| sanitize_line(line));
            let out = format!("{}{}\n", prefix(ctx, url, line.len(), extra), payload);
            let _g = FILE_LOCK.lock().await;
            if let Some(parent) = path.parent() {
                let _ = tokio::fs::create_dir_all(parent).await;
            }
            if let Ok(mut f) = tokio::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .await
            {
                let _ = f.write_all(out.as_bytes()).await;
                let _ = f.flush().await;
            }
        }
    }
}

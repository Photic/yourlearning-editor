use chrono::{Local, NaiveDate};
use std::sync::Mutex;
use tauri::Manager;
use tauri_plugin_opener::OpenerExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

const YOURLEARNING_URL: &str = "https://yourlearning.ibm.com/add-learning";

/// Fixed port the extension always fetches from.
/// Using a single well-known port eliminates the two-port discovery dance.
const RELAY_PORT: u16 = 19421;

// ── YouTube metadata fetch ────────────────────────────────────────────────────

struct VideoMeta {
    title: String,
    author: String,
    duration_secs: u64,
    description: String,
    publish_date: Option<String>,
}

/// Fetches the YouTube watch page and extracts `ytInitialPlayerResponse` from
/// the embedded inline JSON, then returns the fields we need.
async fn fetch_video_meta(url: &str) -> Result<VideoMeta, String> {
    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0 Safari/537.36")
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {e}"))?;

    let html = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("Failed to fetch YouTube page: {e}"))?
        .text()
        .await
        .map_err(|e| format!("Failed to read response body: {e}"))?;

    let json_obj = extract_player_response(&html)
        .ok_or("Could not find ytInitialPlayerResponse in the page. The URL may be invalid.")?;

    let details = &json_obj["videoDetails"];

    let title = details["title"].as_str().unwrap_or("").to_string();
    let author = details["author"].as_str().unwrap_or("").to_string();
    let duration_secs = details["lengthSeconds"]
        .as_str()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);
    let description = details["shortDescription"]
        .as_str()
        .unwrap_or("")
        .to_string();

    // publishDate is under microformat.playerMicroformatRenderer.publishDate
    // as a full ISO 8601 string like "2026-06-29T06:00:36-07:00"; take only
    // the leading "YYYY-MM-DD" portion and reformat to "YYYY/MM/DD".
    let publish_date = json_obj["microformat"]["playerMicroformatRenderer"]["publishDate"]
        .as_str()
        .and_then(|s| NaiveDate::parse_from_str(&s[..s.len().min(10)], "%Y-%m-%d").ok())
        .map(|d| d.format("%Y/%m/%d").to_string());

    Ok(VideoMeta { title, author, duration_secs, description, publish_date })
}

/// Locates `ytInitialPlayerResponse = {...}` in the page HTML and parses the
/// JSON object using a brace counter (avoids pulling in a full HTML parser).
fn extract_player_response(html: &str) -> Option<serde_json::Value> {
    let marker = "ytInitialPlayerResponse = ";
    let start = html.find(marker)? + marker.len();
    let slice = &html[start..];

    let mut depth = 0usize;
    let mut end = 0usize;
    for (i, ch) in slice.char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end = i + 1;
                    break;
                }
            }
            _ => {}
        }
    }

    serde_json::from_str(&slice[..end]).ok()
}

// ── Field processing ──────────────────────────────────────────────────────────

fn format_title(author: &str, title: &str) -> String {
    if author.is_empty() {
        title.to_string()
    } else {
        format!("{author}: {title}")
    }
}

fn split_duration(secs: u64) -> (u64, u64) {
    (secs / 3600, (secs % 3600) / 60)
}

/// Returns the cleaned description, or empty when ≥70% of non-empty lines are
/// link-related (contains a URL or is a short label ≤60 chars).
fn clean_description(raw: &str) -> String {
    let raw = raw.trim();
    let lines: Vec<&str> = raw.lines().collect();
    let non_empty: Vec<&str> = lines.iter().copied().filter(|l| !l.trim().is_empty()).collect();
    let total = non_empty.len().max(1);

    let link_related = non_empty.iter().filter(|l| is_link_related(l)).count();
    if link_related as f64 / total as f64 >= 0.7 {
        return String::new();
    }

    lines
        .iter()
        .filter(|l| !l.contains("http://") && !l.contains("https://"))
        .cloned()
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .chars()
        .take(1000)
        .collect()
}

fn is_link_related(line: &str) -> bool {
    line.contains("http://") || line.contains("https://") || line.trim().len() <= 60
}

// ── Relay HTTP server ─────────────────────────────────────────────────────────

/// State that holds the relay task handle so a previous run can be aborted
/// before we try to bind the same port again.
pub struct RelayState(pub Mutex<Option<JoinHandle<()>>>);

/// Aborts any previous relay task, then binds RELAY_PORT and serves the JSON
/// payload once before shutting down.
async fn run_relay(app: &tauri::AppHandle, json_body: String) -> Result<(), String> {
    // Abort the previous relay task (if any) so its port is released.
    // The lock is dropped before the await so the future stays Send.
    let previous = app.state::<RelayState>().0.lock().unwrap().take();
    if let Some(handle) = previous {
        handle.abort();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    let listener = TcpListener::bind(format!("127.0.0.1:{RELAY_PORT}"))
        .await
        .map_err(|e| format!("Failed to bind relay port {RELAY_PORT}: {e}"))?;

    let handle = tokio::spawn(serve_single(listener, json_body, "application/json"));
    *app.state::<RelayState>().0.lock().unwrap() = Some(handle);

    Ok(())
}

/// Accepts connections on `listener` and responds to every request with
/// `body` and `content_type` until the first successful GET, then stops.
/// OPTIONS preflight requests are answered with a proper CORS response so
/// the browser proceeds to the actual GET.
async fn serve_single(listener: TcpListener, body: String, content_type: &'static str) {
    let cors_preflight =
        "HTTP/1.1 204 No Content\r\n\
         Access-Control-Allow-Origin: *\r\n\
         Access-Control-Allow-Methods: GET, OPTIONS\r\n\
         Access-Control-Allow-Headers: *\r\n\
         Connection: close\r\n\r\n"
        .to_string();

    let data_response = format!(
        "HTTP/1.1 200 OK\r\n\
         Content-Type: {content_type}\r\n\
         Access-Control-Allow-Origin: *\r\n\
         Access-Control-Allow-Methods: GET, OPTIONS\r\n\
         Access-Control-Allow-Headers: *\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\r\n{body}",
        body.len()
    );

    loop {
        if let Ok((mut stream, _)) = listener.accept().await {
            let mut buf = [0u8; 2048];
            if stream.read(&mut buf).await.is_ok() {
                let req = String::from_utf8_lossy(&buf);
                if req.starts_with("OPTIONS") {
                    let _ = stream.write_all(cors_preflight.as_bytes()).await;
                } else if req.starts_with("GET") {
                    let _ = stream.write_all(data_response.as_bytes()).await;
                    break; // Served — shut this listener down
                }
            }
        }
    }
}

// ── Public command ────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn run_add_learning(
    app: tauri::AppHandle,
    url: String,
    date_override: String,
) -> Result<String, String> {
    // Strip extra query params after the video ID (e.g. &t=235s).
    let url = url.trim().splitn(2, '&').next().unwrap_or(&url).to_string();

    // ── Fetch metadata ───────────────────────────────────────────────────────
    let meta = fetch_video_meta(&url).await?;
    let title = format_title(&meta.author, &meta.title);
    let (hours, minutes) = split_duration(meta.duration_secs);
    let description = clean_description(&meta.description);

    // Priority: user override → video publish date → today
    let today = if !date_override.trim().is_empty() {
        // Browser date inputs emit "YYYY-MM-DD"; reformat to "YYYY/MM/DD".
        NaiveDate::parse_from_str(date_override.trim(), "%Y-%m-%d")
            .map(|d| d.format("%Y/%m/%d").to_string())
            .unwrap_or_else(|_| date_override.trim().to_string())
    } else {
        meta.publish_date
            .clone()
            .unwrap_or_else(|| Local::now().format("%Y/%m/%d").to_string())
    };

    let summary = format!(
        "\n{sep}\n  YourLearning — Add Personal Learning\n{sep}\n  Title:    {title}\n  URL:      {url}\n  Duration: {hours}h {minutes}m\n  Date:     {today}\n{sep}\n",
        sep = "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    );

    // ── Start relay server ───────────────────────────────────────────────────
    let json_body = serde_json::json!({
        "title": title,
        "url": url,
        "description": description,
        "today": today,
        "hours": hours,
        "minutes": minutes,
    })
    .to_string();

    run_relay(&app, json_body).await?;

    // ── Open YourLearning in the user's default browser ──────────────────────
    app.opener()
        .open_url(YOURLEARNING_URL, None::<&str>)
        .map_err(|e| format!("Failed to open browser: {e}"))?;

    Ok(format!(
        "{summary}✓ Browser opened — the extension will fill the form automatically."
    ))
}

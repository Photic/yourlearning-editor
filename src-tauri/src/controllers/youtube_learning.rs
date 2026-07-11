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

    // publishDate is under microformat.playerMicroformatRenderer.publishDate
    // as a full ISO 8601 string like "2026-06-29T06:00:36-07:00"; take only
    // the leading "YYYY-MM-DD" portion and reformat to "YYYY/MM/DD".
    let publish_date = json_obj["microformat"]["playerMicroformatRenderer"]["publishDate"]
        .as_str()
        .and_then(|s| NaiveDate::parse_from_str(&s[..s.len().min(10)], "%Y-%m-%d").ok())
        .map(|d| d.format("%Y/%m/%d").to_string());

    Ok(VideoMeta { title, author, duration_secs, publish_date })
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

// ── HF Inference API (bart-large-cnn) ────────────────────────────────────────

/// Returns the HuggingFace API token.
///
/// Priority:
/// 1. Compile-time: `HF_API_TOKEN` baked in via `option_env!` (used in
///    release/DMG builds where no `.env` file is present at runtime).
/// 2. Runtime: `HF_API_TOKEN` env-var or a `.env` file in the working
///    directory (convenient for `cargo tauri dev`).
fn hf_api_token() -> Option<String> {
    // 1. Compile-time value — present when the env-var was set during `cargo build`.
    if let Some(t) = option_env!("HF_API_TOKEN") {
        if !t.is_empty() {
            return Some(t.to_string());
        }
    }
    // 2. Runtime fallback — load .env if present, then check the env-var.
    let _ = dotenvy::dotenv();
    std::env::var("HF_API_TOKEN").ok().filter(|s| !s.is_empty())
}

/// Calls the Hugging Face Inference API to summarise `text` using
/// facebook/bart-large-cnn.  Returns `None` on any error or if no token
/// is available.
///
/// Handles the HF cold-start case: if the model is still loading the API
/// returns `{"error":"Loading…","estimated_time":<secs>}`.  We honour that
/// delay and retry once with `wait_for_model: true`.
async fn summarize_with_bart(text: &str) -> Option<String> {
    let token = match hf_api_token() {
        Some(t) => t,
        None => {
            println!("[HF] HF_API_TOKEN not set in environment or .env — skipping summary.");
            return None;
        }
    };

    // bart-large-cnn has a 1 024-token input limit; truncate conservatively.
    let input: String = text.chars().take(3000).collect();

    let client = reqwest::Client::builder()
        .user_agent("yourlearning-editor")
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .ok()?;

    // First attempt — fast path (model already warm).
    let raw = client
        .post("https://router.huggingface.co/hf-inference/models/facebook/bart-large-cnn")
        .bearer_auth(&token)
        .json(&serde_json::json!({ "inputs": input }))
        .send()
        .await
        .map_err(|e| println!("[HF] Request failed: {e}"))
        .ok()?
        .text()
        .await
        .ok()?;

    println!("[HF] Response: {}", &raw[..raw.len().min(200)]);

    let value: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|e| println!("[HF] JSON parse error: {e}"))
        .ok()?;

    // Model still loading? Wait the suggested delay then retry with
    // wait_for_model so the server blocks until it's ready.
    if value.get("error").is_some() {
        let wait_secs = value["estimated_time"].as_f64().unwrap_or(20.0);
        println!("[HF] Model loading — waiting {wait_secs:.0}s then retrying…");
        tokio::time::sleep(std::time::Duration::from_secs_f64(wait_secs.min(60.0))).await;

        let raw2 = client
            .post("https://router.huggingface.co/hf-inference/models/facebook/bart-large-cnn")
            .bearer_auth(&token)
            .header("X-Wait-For-Model", "true")
            .json(&serde_json::json!({ "inputs": input }))
            .send()
            .await
            .map_err(|e| println!("[HF] Retry request failed: {e}"))
            .ok()?
            .text()
            .await
            .ok()?;

        println!("[HF] Retry response: {}", &raw2[..raw2.len().min(200)]);

        let value2: serde_json::Value = serde_json::from_str(&raw2).ok()?;
        return value2.get(0)?.get("summary_text")?.as_str().map(|s| s.trim().to_string());
    }

    // Successful response: [{"summary_text": "..."}]
    value.get(0)?.get("summary_text")?.as_str().map(|s| s.trim().to_string())
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

/// Fetches captions by replicating yt-dlp's approach:
/// 1. Fetch the watch page to extract visitorData and INNERTUBE_API_KEY
/// 2. POST to /youtubei/v1/player using the Android VR client identity
///    (this is the only client that reliably returns a working timedtext URL)
/// 3. Fetch the timedtext URL from the player response
async fn fetch_captions(video_url: &str) -> Option<String> {
    let video_id = video_url.split("v=").nth(1)
        .and_then(|s| s.split('&').next())?
        .to_string();

    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/15.5 Safari/605.1.15,gzip(gfe)")
        .gzip(true)
        .build()
        .ok()?;

    // Step 1: fetch watch page to get visitorData + API key
    let watch_url = format!("{video_url}&bpctr=9999999999&has_verified=1");
    let html = client
        .get(&watch_url)
        .header("Cookie", "PREF=hl=en&tz=UTC; SOCS=CAI")
        .send()
        .await
        .ok()?
        .text()
        .await
        .ok()?;

    let visitor_data = extract_json_string(&html, "VISITOR_DATA").unwrap_or_default();
    let api_key = extract_json_string(&html, "INNERTUBE_API_KEY").unwrap_or_default();
    println!("[CC] visitor_data: {}...", &visitor_data[..visitor_data.len().min(30)]);
    println!("[CC] api_key: {}...", &api_key[..api_key.len().min(20)]);

    // Step 2: call the internal player API as Android VR — this client returns
    // a timedtext baseUrl whose `ei` token is accepted by YouTube's CDN.
    let player_url = format!(
        "https://www.youtube.com/youtubei/v1/player?prettyPrint=false&key={api_key}"
    );
    let player_body = serde_json::json!({
        "context": {
            "client": {
                "clientName": "ANDROID_VR",
                "clientVersion": "1.65.10",
                "deviceMake": "Oculus",
                "deviceModel": "Quest 3",
                "androidSdkVersion": 32,
                "userAgent": "com.google.android.apps.youtube.vr.oculus/1.65.10 (Linux; U; Android 12L; eureka-user Build/SQ3A.220605.009.A1) gzip",
                "osName": "Android",
                "osVersion": "12L",
                "hl": "en",
                "timeZone": "UTC",
                "utcOffsetMinutes": 0,
                "visitorData": visitor_data,
            }
        },
        "videoId": video_id,
        "playbackContext": {
            "contentPlaybackContext": {
                "html5Preference": "HTML5_PREF_WANTS",
            }
        },
        "contentCheckOk": true,
        "racyCheckOk": true,
    });

    let player_raw = client
        .post(&player_url)
        .header("User-Agent", "com.google.android.apps.youtube.vr.oculus/1.65.10 (Linux; U; Android 12L; eureka-user Build/SQ3A.220605.009.A1) gzip")
        .header("X-Youtube-Client-Name", "28")
        .header("X-Youtube-Client-Version", "1.65.10")
        .header("Origin", "https://www.youtube.com")
        .header("Cookie", "PREF=hl=en&tz=UTC; SOCS=CAI")
        .json(&player_body)
        .send()
        .await
        .ok()?
        .text()
        .await
        .ok()?;

    println!("[CC] player API response ({} bytes): {:.300}", player_raw.len(), player_raw);

    let player_resp: serde_json::Value = serde_json::from_str(&player_raw).ok()?;

    // Step 3: extract the caption track URL and fetch it
    let tracks = &player_resp["captions"]["playerCaptionsTracklistRenderer"]["captionTracks"];
    let base_url = tracks
        .as_array()?
        .iter()
        .find(|t| t["languageCode"].as_str() == Some("en"))
        .or_else(|| tracks.as_array()?.first())?
        .get("baseUrl")?
        .as_str()?
        .to_string();

    println!("[CC] Fetching: {}", &base_url[..base_url.len().min(80)]);

    let xml = client
        .get(&base_url)
        .send()
        .await
        .ok()?
        .text()
        .await
        .ok()?;

    println!("[CC] Response: {} bytes", xml.len());
    if xml.is_empty() {
        return None;
    }

    println!("[CC] XML preview: {}", &xml[..xml.len().min(500)]);
    parse_xml_transcript(&xml)
}

/// Extracts a JSON string value from an HTML page given its key.
/// Handles  "KEY":"value"  patterns embedded in inline JavaScript.
fn extract_json_string(html: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\":\"");
    let start = html.find(&needle)? + needle.len();
    let end = html[start..].find('"')? + start;
    Some(html[start..end].to_string())
}

/// Parses a yt-dlp json3 caption file into plain text.
/// json3 format: { "events": [ { "segs": [ { "utf8": "text" } ] } ] }
/// Events with "aAppend":1 are mid-word continuations — skip their leading newline.
fn parse_json3_transcript(body: &str) -> Option<String> {
    let json: serde_json::Value = serde_json::from_str(body).ok()?;
    let events = json["events"].as_array()?;

    let mut transcript = String::new();
    for event in events {
        // Skip window-config events (no segs)
        let segs = match event["segs"].as_array() {
            Some(s) => s,
            None => continue,
        };
        for seg in segs {
            if let Some(text) = seg["utf8"].as_str() {
                // Replace newlines with spaces; yt-dlp uses \n as word separators
                let text = text.replace('\n', " ");
                let text = text.trim();
                if !text.is_empty() {
                    if !transcript.is_empty() {
                        transcript.push(' ');
                    }
                    transcript.push_str(text);
                }
            }
        }
    }

    if transcript.is_empty() { None } else { Some(transcript) }
}

/// Parses the timedtext format="3" XML returned by the YouTube player API.
///
/// Two formats are encountered in the wild:
/// - Modern ASR captions: `<p>` contains `<s>` word segments.
/// - Older / manual captions: text sits directly inside `<p>` with no children.
///
/// We try the `<s>`-segment path first; if that yields nothing we fall back to
/// stripping all child tags from `<p>` and using the raw text content.
fn parse_xml_transcript(xml: &str) -> Option<String> {
    // Find <body> — everything before it is header metadata we skip.
    let body_start = xml.find("<body>")?;
    let body = &xml[body_start..];

    let mut transcript = String::new();

    // Iterate over every <p …> … </p> block.
    let mut remaining = body;
    while let Some(p_open) = remaining.find("<p ").or_else(|| remaining.find("<p>")) {
        remaining = &remaining[p_open..];
        let p_close = match remaining.find("</p>") {
            Some(i) => i,
            None => break,
        };
        let p_block = &remaining[..p_close + 4];

        // ── Path 1: collect all <s …>text</s> segments ───────────────────────
        let mut para = String::new();
        let mut seg = p_block;
        while let Some(s_open) = seg.find("<s") {
            seg = &seg[s_open..];
            let content_start = match seg.find('>') {
                Some(i) => i + 1,
                None => break,
            };
            // Self-closing <s … /> — no text content, skip.
            if seg[..content_start].ends_with("/>") {
                seg = &seg[content_start..];
                continue;
            }
            let content_end = match seg.find("</s>") {
                Some(i) => i,
                None => break,
            };
            let word = decode_xml_entities(&seg[content_start..content_end]);
            para.push_str(&word);
            seg = &seg[content_end + 4..];
        }

        // ── Path 2: no <s> segments — text is directly inside <p> ────────────
        // Skip past the opening <p …> tag, then strip any remaining child tags.
        if para.is_empty() {
            if let Some(tag_end) = p_block.find('>') {
                let inner = &p_block[tag_end + 1..];
                // Strip everything that looks like a tag.
                let mut raw = String::new();
                let mut inside_tag = false;
                for ch in inner.chars() {
                    match ch {
                        '<' => inside_tag = true,
                        '>' => inside_tag = false,
                        _ if !inside_tag => raw.push(ch),
                        _ => {}
                    }
                }
                para = decode_xml_entities(raw.replace('\n', " ").trim());
            }
        }

        let para = para.trim().to_string();
        if !para.is_empty() {
            if !transcript.is_empty() {
                transcript.push(' ');
            }
            transcript.push_str(&para);
        }

        remaining = &remaining[p_close + 4..];
    }

    if transcript.is_empty() { None } else { Some(transcript) }
}

/// Decodes the XML character entities used in YouTube timedtext responses.
fn decode_xml_entities(s: &str) -> String {
    s.replace("&amp;", "&")
     .replace("&lt;", "<")
     .replace("&gt;", ">")
     .replace("&quot;", "\"")
     .replace("&#39;", "'")
     .replace("&apos;", "'")
     .replace('\n', " ")
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
    }

    // Retry binding: the OS may not release the port immediately after abort.
    let listener = {
        let mut last_err = String::new();
        let mut listener = None;
        for attempt in 0..10 {
            if attempt > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
            match TcpListener::bind(format!("127.0.0.1:{RELAY_PORT}")).await {
                Ok(l) => { listener = Some(l); break; }
                Err(e) => { last_err = format!("Failed to bind relay port {RELAY_PORT}: {e}"); }
            }
        }
        listener.ok_or(last_err)?
    };

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

    // ── Fetch captions via yt-dlp and summarise ──────────────────────────────
    let description = {
        let transcript = fetch_captions(&url).await;
        match transcript {
            Some(text) => {
                println!("[CC] Transcript ({} chars):\n{text}\n", text.len());
                let summary = summarize_with_bart(&text).await;
                println!("[CC] Summary: {summary:?}");
                summary.unwrap_or_default()
            }
            None => {
                println!("[CC] No captions found (yt-dlp not installed or video has no CC)");
                String::new()
            }
        }
    };

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

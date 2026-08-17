use super::common::{compute_lix, finish_add_learning, lix_label, split_duration, summarize_with_bart, transcript_stats};
use crate::{http, storage};
use chrono::{Local, NaiveDate};
use dioxus_logger::tracing;

// ── YouTube metadata fetch ────────────────────────────────────────────────────

struct VideoMeta {
    title: String,
    author: String,
    duration_secs: u64,
    publish_date: Option<String>,
}

const YOUTUBE_UA: &str =
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0 Safari/537.36";

/// Fetches the YouTube watch page and extracts `ytInitialPlayerResponse` from
/// the embedded inline JSON, then returns the fields we need.
///
/// Note: fetch() can't override the User-Agent header (it's a forbidden
/// header name in browser contexts), so the real extension UA is sent
/// instead — harmless here since YouTube's watch-page HTML doesn't vary on it.
async fn fetch_video_meta(url: &str) -> Result<VideoMeta, String> {
    let html = http::get(url, &[("User-Agent", YOUTUBE_UA)], 30_000)
        .await
        .map_err(|e| format!("Failed to fetch YouTube page: {e}"))?;

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

// ── Field processing ──────────────────────────────────────────────────────────

fn format_title(author: &str, title: &str) -> String {
    if author.is_empty() {
        title.to_string()
    } else {
        format!("{author}: {title}")
    }
}

/// Extracts the YouTube video ID from either format:
/// - `https://www.youtube.com/watch?v=VIDEO_ID`
/// - `https://youtu.be/VIDEO_ID`
fn extract_video_id(url: &str) -> Option<String> {
    if let Some(rest) = url.split("v=").nth(1) {
        // Standard watch URL: take everything up to the next '&'
        return Some(rest.split('&').next()?.to_string());
    }
    if let Some(rest) = url.split("youtu.be/").nth(1) {
        // Short URL: video ID is the path segment before any '?' or '&'
        return Some(rest.split(|c| c == '?' || c == '&').next()?.to_string());
    }
    None
}

/// Fetches captions by replicating yt-dlp's approach:
/// 1. Fetch the watch page to extract visitorData and INNERTUBE_API_KEY
/// 2. POST to /youtubei/v1/player using the Android VR client identity
///    (this is the only client that reliably returns a working timedtext URL)
/// 3. Fetch the timedtext URL from the player response
///
/// Note: the `Cookie` and `Origin` headers below are forbidden header names
/// in browser `fetch()` and are silently dropped by the browser — this flow
/// impersonated a native yt-dlp-style client via those headers, so caption
/// fetching may be less reliable here than it was from the native app.
async fn fetch_captions(video_url: &str) -> Option<String> {
    let video_id = extract_video_id(video_url)?;

    // Step 1: fetch watch page to get visitorData + API key
    let watch_url = format!("{video_url}&bpctr=9999999999&has_verified=1");
    let html = http::get(&watch_url, &[("Cookie", "PREF=hl=en&tz=UTC; SOCS=CAI")], 30_000)
        .await
        .ok()?;

    let visitor_data = extract_json_string(&html, "VISITOR_DATA").unwrap_or_default();
    let api_key = extract_json_string(&html, "INNERTUBE_API_KEY").unwrap_or_default();
    tracing::debug!("[CC] visitor_data: {}...", &visitor_data[..visitor_data.len().min(30)]);
    tracing::debug!("[CC] api_key: {}...", &api_key[..api_key.len().min(20)]);

    // Step 2: call the internal player API as Android VR — this client returns
    // a timedtext baseUrl whose `ei` token is accepted by YouTube's CDN.
    let player_url = format!("https://www.youtube.com/youtubei/v1/player?prettyPrint=false&key={api_key}");
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

    let player_raw = http::post_json(
        &player_url,
        &[
            ("X-Youtube-Client-Name", "28"),
            ("X-Youtube-Client-Version", "1.65.10"),
            ("Origin", "https://www.youtube.com"),
            ("Cookie", "PREF=hl=en&tz=UTC; SOCS=CAI"),
        ],
        &player_body,
        30_000,
    )
    .await
    .ok()?;

    tracing::debug!("[CC] player API response ({} bytes): {:.300}", player_raw.len(), player_raw);

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

    tracing::debug!("[CC] Fetching: {}", &base_url[..base_url.len().min(80)]);

    let xml = http::get(&base_url, &[], 30_000).await.ok()?;

    tracing::debug!("[CC] Response: {} bytes", xml.len());
    if xml.is_empty() {
        return None;
    }

    tracing::debug!("[CC] XML preview: {}", &xml[..xml.len().min(500)]);
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

// ── Public handler ────────────────────────────────────────────────────────────

pub(crate) async fn run_youtube_learning(
    url: &str,
    date_override: &str,
    use_ai_summary: bool,
) -> Result<(), String> {
    // Strip extra query params after the video ID (e.g. &t=235s).
    let url = url.splitn(2, '&').next().unwrap_or(url);

    // ── Fetch metadata ───────────────────────────────────────────────────────
    let meta = fetch_video_meta(url).await?;
    let title = format_title(&meta.author, &meta.title);
    let (hours, minutes) = split_duration(meta.duration_secs);

    // ── Fetch captions and summarise ─────────────────────────────────────────
    let (description, analytics) = if use_ai_summary {
        let transcript = fetch_captions(url).await;
        match transcript {
            Some(text) => {
                tracing::debug!("[CC] Transcript ({} chars):\n{text}\n", text.len());
                let summary_result = summarize_with_bart(&text).await;
                tracing::debug!("[CC] Summary: {summary_result:?}");

                let lix = compute_lix(&text);
                let (words, read_mins) = transcript_stats(&text);

                let (description, warning) = match summary_result {
                    Ok(Some(s)) => (s, None),
                    Ok(None) => (String::new(), None),
                    Err(e) => (String::new(), Some(e)),
                };

                let info = storage::AnalyticsInfo {
                    primary_label: "Transcript".to_string(),
                    primary_value: format!("{words} words  |  ~{read_mins} min read"),
                    lix: lix.map(|score| storage::LixScore { score, label: lix_label(score).to_string() }),
                    warning,
                };

                (description, Some(info))
            }
            None => {
                tracing::debug!("[CC] No captions found");
                (String::new(), None)
            }
        }
    } else {
        (String::new(), None)
    };

    // Priority: user override → video publish date → today
    let today = if !date_override.trim().is_empty() {
        NaiveDate::parse_from_str(date_override.trim(), "%Y-%m-%d")
            .map(|d| d.format("%Y/%m/%d").to_string())
            .unwrap_or_else(|_| date_override.trim().to_string())
    } else {
        meta.publish_date
            .unwrap_or_else(|| Local::now().format("%Y/%m/%d").to_string())
    };

    finish_add_learning(url, &title, hours, minutes, &today, &description, analytics).await
}

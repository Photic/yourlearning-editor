use chrono::{Local, NaiveDate};

use super::youtube_learning::{
    finish_add_learning, hf_api_token, split_duration, summarize_with_bart, transcript_stats,
    compute_lix, lix_label,
};

// ── URL helpers ───────────────────────────────────────────────────────────────

/// Extracts the bare episode ID from a Spotify episode URL.
/// Handles both:
///   https://open.spotify.com/episode/{id}
///   https://open.spotify.com/episode/{id}?si=...
fn parse_episode_id(url: &str) -> Option<&str> {
    let after = url.split("/episode/").nth(1)?;
    Some(after.split('?').next().unwrap_or(after))
}

// ── HTTP helpers ──────────────────────────────────────────────────────────────

async fn http_get(url: &str) -> Option<String> {
    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .ok()?;

    client
        .get(url)
        .send()
        .await
        .ok()?
        .text()
        .await
        .ok()
}

// ── Metadata extraction ───────────────────────────────────────────────────────

struct SpotifyEpisodeMeta {
    title: String,
    show_name: String,
    duration_ms: u64,
    release_date: String, // ISO 8601: "2026-07-10T18:29:00Z"
    description: String,
}

struct JinaMeta {
    title: Option<String>,
    description: Option<String>,
}

/// Fetches title and episode description from Jina's reader.
///
/// Jina returns a Markdown document structured as:
///   Title: <episode title> - <show name>
///   ...
///   ## Episode Description
///   <description text>
///   [See all episodes](...)
///   <more description text>
///   Apr 1          ← first date line = start of next episode listing
///   ...
///
/// We extract the `Title:` line and all text under `## Episode Description`
/// up to (but not including) the first date line.
///
/// Important: do NOT send a browser User-Agent — Jina forwards it to Spotify,
/// which then returns a 403. Use Jina's default (no User-Agent header).
async fn fetch_via_jina(episode_url: &str) -> Option<JinaMeta> {
    let jina_url = format!("https://r.jina.ai/{episode_url}");
    println!("[Spotify] Calling Jina: {jina_url}");
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .ok()?;
    let body = client.get(&jina_url).send().await.ok()?.text().await.ok()?;
    println!("[Spotify] Jina response length: {} chars", body.len());

    // ── Title: first line that starts with "Title: " ──────────────────────────
    let title = body.lines()
        .find(|l| l.starts_with("Title: "))
        .map(|l| l["Title: ".len()..].trim().to_string())
        .filter(|s| !s.is_empty());

    // ── Description: text after "## Episode Description" up to first date line ─
    let section_marker = "## Episode Description";
    let description = body.split(section_marker).nth(1).map(|after_header| {
        after_header
            .lines()
            .take_while(|line| {
                // Date lines look like "Apr 1", "Jun 30", "Jan 27", etc.
                let parts: Vec<&str> = line.split_whitespace().collect();
                !matches!(parts.as_slice(),
                    [month, ..] if matches!(*month,
                        "Jan" | "Feb" | "Mar" | "Apr" | "May" | "Jun" |
                        "Jul" | "Aug" | "Sep" | "Oct" | "Nov" | "Dec"
                    )
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
            .trim()
            .to_string()
    })
    .filter(|s| !s.is_empty());

    println!("[Spotify] Jina title: {title:?}");
    println!("[Spotify] Jina description ({} chars): {:?}",
        description.as_deref().unwrap_or("").len(),
        description.as_deref().unwrap_or("").chars().take(120).collect::<String>());

    Some(JinaMeta { title, description })
}

/// Pulls episode metadata from two unauthenticated sources:
///
/// 1. `__NEXT_DATA__` JSON inside `open.spotify.com/embed/episode/{id}`:
///    provides duration_ms, releaseDate, subtitle (show name).
///
/// 2. JSON-LD `<script type="application/ld+json">` inside `open.spotify.com/episode/{id}`:
///    provides description, datePublished, and episode name.
///
/// 3. Jina reader (`r.jina.ai`) as a fallback for the description when the
///    above sources return an empty or missing description.
///
/// Sources 1 and 2 are fired concurrently; Jina is only called when needed.
async fn fetch_episode_meta(episode_id: &str) -> Option<SpotifyEpisodeMeta> {
    let embed_url = format!("https://open.spotify.com/embed/episode/{episode_id}");
    let page_url  = format!("https://open.spotify.com/episode/{episode_id}");

    let embed_future = http_get(&embed_url);
    let page_future  = http_get(&page_url);
    let embed_html = embed_future.await;
    let page_html  = page_future.await;

    // ── Parse __NEXT_DATA__ from the embed page ───────────────────────────────
    let (title_from_embed, show_name, duration_ms, release_date) =
        parse_next_data(embed_html.as_deref().unwrap_or(""));

    // ── Parse JSON-LD from the episode page ───────────────────────────────────
    let (title_from_ld, description, date_from_ld) =
        parse_json_ld(page_html.as_deref().unwrap_or(""));

    let release_date = release_date.or(date_from_ld).unwrap_or_default();

    // ── Fall back to Jina for title and/or description when needed ────────────
    // A corrupted title contains "__" (CSS class name leaked from __NEXT_DATA__).
    let title_looks_bad = |t: &str| t.contains("__") || t.is_empty();
    let need_jina = description.as_deref().map_or(true, str::is_empty)
        || title_from_ld.as_deref().map_or(true, title_looks_bad)
        || title_from_embed.as_deref().map_or(true, title_looks_bad);

    let jina = if need_jina {
        println!("[Spotify] Falling back to Jina for missing/corrupt metadata…");
        let episode_url = format!("https://open.spotify.com/episode/{episode_id}");
        fetch_via_jina(&episode_url).await
    } else {
        None
    };

    // Prefer clean scraped title; fall back to Jina title (which already
    // contains the show name appended as " - Show"), then to embed title.
    let jina_title_used;
    let title = if let Some(t) = title_from_ld.filter(|t| !title_looks_bad(t)) {
        jina_title_used = false;
        t
    } else if let Some(t) = jina.as_ref().and_then(|j| j.title.clone()) {
        jina_title_used = true;
        t
    } else {
        jina_title_used = false;
        title_from_embed.filter(|t| !title_looks_bad(t)).unwrap_or_default()
    };

    // When using the Jina title it already contains the show name, so clear
    // show_name to prevent it being prepended a second time.
    let show_name = if jina_title_used {
        String::new()
    } else {
        show_name.filter(|s| !title_looks_bad(s)).unwrap_or_default()
    };

    let description = description
        .filter(|d| !d.is_empty())
        .or_else(|| jina.and_then(|j| j.description))
        .unwrap_or_default();

    Some(SpotifyEpisodeMeta {
        title,
        show_name,
        duration_ms,
        release_date,
        description,
    })
}

/// Extracts (title, show_name, duration_ms, release_date_iso) from the
/// `__NEXT_DATA__` JSON block embedded in Spotify's embed page.
fn parse_next_data(html: &str) -> (Option<String>, Option<String>, u64, Option<String>) {
    let marker = "\"duration\":";
    if !html.contains(marker) {
        return (None, None, 0, None);
    }

    // Extract the value after "duration":
    let duration_ms = html.split("\"duration\":").nth(1)
        .and_then(|s| s.split([',', '}']).next())
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(0);

    let show_name = extract_json_string(html, "subtitle");
    let title     = extract_json_string(html, "name");
    let date      = extract_json_string(html, "isoString");

    (title, show_name, duration_ms, date)
}

/// Extracts (title, description, datePublished) from the JSON-LD block in the
/// episode page HTML.
fn parse_json_ld(html: &str) -> (Option<String>, Option<String>, Option<String>) {
    let Some(start) = html.find("<script type=\"application/ld+json\">") else {
        return (None, None, None);
    };
    let rest = &html[start + 35..];
    let Some(end) = rest.find("</script>") else {
        return (None, None, None);
    };
    let json = &rest[..end];

    let title       = extract_json_string(json, "name");
    let description = extract_json_string(json, "description");
    let date        = extract_json_string(json, "datePublished");

    (title, description, date)
}

/// Naively extracts the string value for a JSON key from raw JSON text.
/// Handles both `"key":"value"` and `"key": "value"` forms.
fn extract_json_string(json: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let after  = json.split(&needle).nth(1)?;
    let colon  = after.find('"')?;
    let value_start = colon + 1; // skip the opening quote
    let value  = &after[value_start..];
    let end    = value.find('"')?;
    let s = value[..end].replace("\\u0026", "&").replace("\\/", "/");
    if s.is_empty() { None } else { Some(s) }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Truncates a description to 1000 characters at a word boundary and appends `…`.
fn truncate_description(s: &str) -> String {
    // char_indices gives us valid UTF-8 boundaries; take up to 1000 chars.
    let cut = s.char_indices()
        .nth(1000)
        .map(|(i, _)| i)
        .unwrap_or(s.len());
    let trimmed = s[..cut].trim_end();
    let end = trimmed.rfind(' ').unwrap_or(trimmed.len());
    format!("{}…", trimmed[..end].trim_end())
}

// ── Public handler ────────────────────────────────────────────────────────────

/// Handles a Spotify episode URL (`open.spotify.com/episode/…`).
///
/// Fetches metadata from the embed page and episode page (both unauthenticated),
/// then follows the same pattern as the Apple and RSS podcast handlers.
pub(crate) async fn run_spotify_podcast(
    app: &tauri::AppHandle,
    url: &str,
    date_override: &str,
    use_ai_summary: bool,
) -> Result<String, String> {
    println!("[Spotify] Fetching metadata for {url}");

    let episode_id = parse_episode_id(url)
        .ok_or_else(|| "Could not extract episode ID from the Spotify URL.".to_string())?;

    let meta = fetch_episode_meta(episode_id)
        .await
        .ok_or_else(|| "Failed to fetch metadata from Spotify.".to_string())?;

    let title = if meta.show_name.is_empty() {
        meta.title.clone()
    } else {
        format!("{}: {}", meta.show_name, meta.title)
    };

    println!("[Spotify] Title: {title:?}");
    println!("[Spotify] Duration: {} ms, Date: {:?}", meta.duration_ms, meta.release_date);

    let (hours, minutes) = split_duration(meta.duration_ms / 1000);

    let date = if !date_override.trim().is_empty() {
        NaiveDate::parse_from_str(date_override.trim(), "%Y-%m-%d")
            .map(|d| d.format("%Y/%m/%d").to_string())
            .unwrap_or_else(|_| date_override.trim().to_string())
    } else if !meta.release_date.is_empty() {
        // ISO 8601 — take the date part only
        NaiveDate::parse_from_str(&meta.release_date[..10], "%Y-%m-%d")
            .map(|d| d.format("%Y/%m/%d").to_string())
            .unwrap_or_else(|_| Local::now().format("%Y/%m/%d").to_string())
    } else {
        Local::now().format("%Y/%m/%d").to_string()
    };

    let mut hf_warning: Option<String> = None;
    let description = if meta.description.len() > 1000 {
        if use_ai_summary && hf_api_token(app).is_some() {
            let summary_result = summarize_with_bart(app, &meta.description).await;
            println!("[Spotify] Summary: {summary_result:?}");
            match summary_result {
                Ok(Some(s)) => s,
                Ok(None) => truncate_description(&meta.description),
                Err(e) => { hf_warning = Some(e); truncate_description(&meta.description) }
            }
        } else {
            truncate_description(&meta.description)
        }
    } else {
        meta.description.clone()
    };

    let transcript_info = if meta.description.trim().is_empty() {
        None
    } else {
        let (words, read_mins) = transcript_stats(&meta.description);
        let lix = compute_lix(&meta.description);
        let mut info = match lix {
            Some(score) => format!(
                "  Description: {} words  |  ~{} min read\n  LIX score:   {:.1} — {}",
                words, read_mins, score, lix_label(score)
            ),
            None => format!("  Description: {} words  |  ~{} min read", words, read_mins),
        };
        if let Some(w) = hf_warning { info.push_str(&format!("\n  ⚠ AI summary: {w}")); }
        Some(info)
    };

    finish_add_learning(app, url, &title, hours, minutes, &date, &description, transcript_info)
        .await
}

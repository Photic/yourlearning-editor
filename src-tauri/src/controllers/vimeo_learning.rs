use chrono::{Local, NaiveDate};

use super::youtube_learning::{
    finish_add_learning, hf_api_token, split_duration, summarize_with_bart, transcript_stats,
    compute_lix, lix_label,
};

// ── oEmbed fetch ──────────────────────────────────────────────────────────────

struct VimeoMeta {
    title: String,
    author_name: String,
    duration_secs: u64,
    upload_date: String, // "YYYY-MM-DD" or empty
    description: String,
}

/// Fetches Vimeo's public oEmbed endpoint for a video URL.
/// Returns title, author, duration (seconds), upload_date, and description.
///
/// oEmbed endpoint: `https://vimeo.com/api/oembed.json?url=<encoded_url>`
/// This is unauthenticated and always public.
async fn fetch_oembed(url: &str) -> Option<VimeoMeta> {
    let encoded = url.replace('&', "%26");
    let oembed_url = format!("https://vimeo.com/api/oembed.json?url={encoded}");

    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (compatible; yourlearning-editor)")
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .ok()?;

    let json: serde_json::Value = client
        .get(&oembed_url)
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()?;

    println!("[Vimeo] oEmbed response: title={:?} duration={:?}",
        json["title"].as_str(), json["duration"].as_u64());

    let title       = json["title"].as_str().unwrap_or("").to_string();
    let author_name = json["author_name"].as_str().unwrap_or("").to_string();
    let duration_secs = json["duration"].as_u64().unwrap_or(0);
    let upload_date = json["upload_date"].as_str().unwrap_or("").to_string();
    let description = json["description"].as_str().unwrap_or("").to_string();

    if title.is_empty() && duration_secs == 0 {
        return None;
    }

    Some(VimeoMeta { title, author_name, duration_secs, upload_date, description })
}

// ── Public handler ────────────────────────────────────────────────────────────

/// Handles a Vimeo URL (`vimeo.com/<id>` or `player.vimeo.com/video/<id>`).
///
/// Metadata is fetched from Vimeo's unauthenticated oEmbed API — no API key
/// or login required.  The upload_date from oEmbed is used as the default date.
pub(crate) async fn run_vimeo(
    app: &tauri::AppHandle,
    url: &str,
    date_override: &str,
    use_ai_summary: bool,
) -> Result<String, String> {
    println!("[Vimeo] Fetching metadata for {url}");

    let meta = fetch_oembed(url)
        .await
        .ok_or_else(|| "Failed to fetch metadata from Vimeo oEmbed API. The video may be private.".to_string())?;

    let title = if meta.author_name.is_empty() {
        meta.title.clone()
    } else {
        format!("{}: {}", meta.author_name, meta.title)
    };

    println!("[Vimeo] Title: {title:?}, Duration: {}s", meta.duration_secs);

    let (hours, minutes) = split_duration(meta.duration_secs);

    // Priority: user override → oEmbed upload_date → today
    let date = if !date_override.trim().is_empty() {
        NaiveDate::parse_from_str(date_override.trim(), "%Y-%m-%d")
            .map(|d| d.format("%Y/%m/%d").to_string())
            .unwrap_or_else(|_| date_override.trim().to_string())
    } else if !meta.upload_date.is_empty() {
        // oEmbed returns "YYYY-MM-DD HH:MM:SS" or "YYYY-MM-DD"
        let date_part = &meta.upload_date[..10.min(meta.upload_date.len())];
        NaiveDate::parse_from_str(date_part, "%Y-%m-%d")
            .map(|d| d.format("%Y/%m/%d").to_string())
            .unwrap_or_else(|_| Local::now().format("%Y/%m/%d").to_string())
    } else {
        Local::now().format("%Y/%m/%d").to_string()
    };

    // Optionally summarise the description
    let mut hf_warning: Option<String> = None;
    let description = if !meta.description.is_empty() && use_ai_summary && hf_api_token(app).is_some() {
        let summary_result = summarize_with_bart(app, &meta.description).await;
        println!("[Vimeo] Summary: {summary_result:?}");
        match summary_result {
            Ok(Some(s)) => s,
            Ok(None) => meta.description.clone(),
            Err(e) => { hf_warning = Some(e); meta.description.clone() }
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

    finish_add_learning(app, url, &title, hours, minutes, &date, &description, transcript_info).await
}

use chrono::{Local, NaiveDate};

use super::youtube_learning::{
    finish_add_learning, hf_api_token, split_duration, summarize_with_bart, transcript_stats,
    compute_lix, lix_label,
};

// ── iTunes Lookup API ─────────────────────────────────────────────────────────

/// Extracts the numeric Apple Podcasts ID from a URL of the form
/// `https://podcasts.apple.com/…/idNNNNNNNNN` (show or episode page).
/// Also extracts the episode ID from the `?i=NNNNN` query parameter when present.
fn parse_apple_url(url: &str) -> Option<(String, Option<String>)> {
    // Show/podcast ID: the path segment starting with "id"
    let podcast_id = url
        .split('/')
        .find(|seg| seg.starts_with("id") && seg[2..].chars().all(|c| c.is_ascii_digit()))?
        .trim_start_matches("id")
        .to_string();

    // Optional episode ID from `?i=...` query parameter
    let episode_id = url
        .split('?')
        .nth(1)
        .and_then(|qs| {
            qs.split('&')
                .find(|p| p.starts_with("i="))
                .map(|p| p[2..].to_string())
        });

    Some((podcast_id, episode_id))
}

/// Calls the iTunes Lookup API for a podcast show or episode.
/// Returns the raw JSON `Value` of the first (and usually only) result.
async fn itunes_lookup(id: &str, entity: &str) -> Option<serde_json::Value> {
    let url = format!(
        "https://itunes.apple.com/lookup?id={id}&entity={entity}&limit=1"
    );
    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (compatible; yourlearning-editor)")
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .ok()?;

    let text = client
        .get(&url)
        .send()
        .await
        .ok()?
        .text()
        .await
        .ok()?;

    let json: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| println!("[Apple] iTunes parse error: {e}"))
        .ok()?;

    // The API returns { "resultCount": N, "results": [...] }
    // Index 0 is the show itself (wrapperType=podcast), index 1 onward are episodes.
    // For an episode lookup we want the result with wrapperType == "podcastEpisode".
    let results = json["results"].as_array()?;
    results.first().cloned()
}

/// Looks up a specific episode by its episode ID using the podcast's feed URL.
/// The iTunes Search API does not support direct episode lookup by episode ID
/// reliably, so we fetch the RSS feed and find the episode by its GUID or by
/// matching the numeric episode ID embedded in the feed item URL.
async fn lookup_episode_from_feed(
    feed_url: &str,
    episode_id: &str,
) -> Option<(String, u64, String, String)> {
    // episode_id here is the numeric string from ?i=NNNNN
    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (compatible; yourlearning-editor)")
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .ok()?;

    let xml = client
        .get(feed_url)
        .send()
        .await
        .ok()?
        .text()
        .await
        .ok()?;

    // Walk every <item> and look for the one whose enclosure/guid contains the
    // episode_id string (Apple embeds the episode ID in the episode GUID or URL).
    parse_rss_episode_by_id(&xml, episode_id)
}

/// Parses an RSS feed XML string and finds the episode whose `<guid>` or any
/// enclosure URL contains `episode_id`.  Returns (title, duration_secs, pub_date, description).
fn parse_rss_episode_by_id(
    xml: &str,
    episode_id: &str,
) -> Option<(String, u64, String, String)> {
    let lower = xml.to_lowercase();
    let mut pos = 0;

    while let Some(item_start) = lower[pos..].find("<item>").map(|i| pos + i) {
        let item_end = lower[item_start..]
            .find("</item>")
            .map(|i| item_start + i + 7)?;
        let item = &xml[item_start..item_end];
        let item_lower = &lower[item_start..item_end];

        if item_lower.contains(episode_id) {
            let title = extract_xml_text(item, "title").unwrap_or_default();
            let duration_secs = extract_xml_text(item, "itunes:duration")
                .as_deref()
                .map(parse_duration_str)
                .unwrap_or(0);
            let pub_date = extract_xml_text(item, "pubDate")
                .as_deref()
                .map(parse_rfc2822_date)
                .unwrap_or_default();
            let description = extract_xml_text(item, "itunes:summary")
                .or_else(|| extract_xml_text(item, "description"))
                .unwrap_or_default();
            return Some((title, duration_secs, pub_date, description));
        }

        pos = item_end;
    }

    None
}

// ── XML helpers ───────────────────────────────────────────────────────────────

/// Returns the text content of the first `<tag>…</tag>` pair (case-insensitive).
/// Handles CDATA sections (`<![CDATA[…]]>`).
fn extract_xml_text(xml: &str, tag: &str) -> Option<String> {
    let lower = xml.to_lowercase();
    let open = format!("<{}>", tag.to_lowercase());
    let close = format!("</{}>", tag.to_lowercase());

    // Also handle <tag attr="…">
    let tag_start = lower.find(&open).or_else(|| {
        let prefix = format!("<{} ", tag.to_lowercase());
        lower.find(&prefix)
    })?;

    let content_start = xml[tag_start..].find('>')? + tag_start + 1;
    let content_end = lower[content_start..].find(&close)? + content_start;

    let raw = xml[content_start..content_end].trim();

    // Strip CDATA wrapper if present
    let text = if raw.starts_with("<![CDATA[") && raw.ends_with("]]>") {
        raw[9..raw.len() - 3].trim().to_string()
    } else {
        decode_xml_entities(raw)
    };

    if text.is_empty() { None } else { Some(text) }
}

fn decode_xml_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ")
}

/// Parses an itunes:duration value.
/// Accepts "HH:MM:SS", "MM:SS", or plain seconds as a string.
fn parse_duration_str(s: &str) -> u64 {
    let parts: Vec<u64> = s
        .split(':')
        .filter_map(|p| p.trim().parse::<u64>().ok())
        .collect();
    match parts.as_slice() {
        [h, m, s] => h * 3600 + m * 60 + s,
        [m, s] => m * 60 + s,
        [s] => *s,
        _ => 0,
    }
}

/// Parses an RFC 2822 date string (e.g. "Mon, 17 Jun 2024 10:00:00 +0000")
/// and returns a "YYYY/MM/DD" string.  Falls back to today on parse error.
fn parse_rfc2822_date(s: &str) -> String {
    // chrono supports RFC 2822 via DateTime::parse_from_rfc2822
    chrono::DateTime::parse_from_rfc2822(s.trim())
        .map(|dt| dt.format("%Y/%m/%d").to_string())
        .unwrap_or_else(|_| Local::now().format("%Y/%m/%d").to_string())
}

// ── Public handler ────────────────────────────────────────────────────────────

/// Handles an Apple Podcasts URL:
/// - Looks up show + optional episode via the iTunes API.
/// - If an episode is specified, tries to find a description from the RSS feed.
/// - Optionally summarises the description with BART.
pub(crate) async fn run_apple_podcast(
    app: &tauri::AppHandle,
    url: &str,
    date_override: &str,
    use_ai_summary: bool,
) -> Result<String, String> {
    println!("[Apple] Fetching metadata for {url}");

    let (podcast_id, episode_id) = parse_apple_url(url)
        .ok_or_else(|| "Could not extract podcast ID from the Apple Podcasts URL.".to_string())?;

    println!("[Apple] Podcast ID: {podcast_id}, Episode ID: {episode_id:?}");

    // Always look up the show first to get the feed URL and show name.
    let show = itunes_lookup(&podcast_id, "podcast")
        .await
        .ok_or_else(|| "iTunes API returned no results for this podcast.".to_string())?;

    let show_name = show["collectionName"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let artist = show["artistName"].as_str().unwrap_or("").to_string();
    let feed_url = show["feedUrl"].as_str().unwrap_or("").to_string();

    println!("[Apple] Show: {show_name:?}, Artist: {artist:?}");

    // ── Episode path ─────────────────────────────────────────────────────────
    if let Some(ref ep_id) = episode_id {
        println!("[Apple] Looking up episode {ep_id} from feed {feed_url}");

        let episode = if !feed_url.is_empty() {
            lookup_episode_from_feed(&feed_url, ep_id).await
        } else {
            None
        };

        if let Some((ep_title, duration_secs, pub_date_str, ep_desc)) = episode {
            let title = if show_name.is_empty() {
                ep_title.clone()
            } else {
                format!("{show_name}: {ep_title}")
            };

            let (hours, minutes) = split_duration(duration_secs);

            let date = if !date_override.trim().is_empty() {
                NaiveDate::parse_from_str(date_override.trim(), "%Y-%m-%d")
                    .map(|d| d.format("%Y/%m/%d").to_string())
                    .unwrap_or_else(|_| date_override.trim().to_string())
            } else if !pub_date_str.is_empty() {
                pub_date_str.clone()
            } else {
                Local::now().format("%Y/%m/%d").to_string()
            };

            // Optionally summarise the episode description
            let mut hf_warning: Option<String> = None;
            let description = if use_ai_summary && hf_api_token(app).is_some() && !ep_desc.is_empty() {
                let summary_result = summarize_with_bart(app, &ep_desc).await;
                println!("[Apple] Summary: {summary_result:?}");
                match summary_result {
                    Ok(Some(s)) => s,
                    Ok(None) => ep_desc.clone(),
                    Err(e) => { hf_warning = Some(e); ep_desc.clone() }
                }
            } else {
                ep_desc.clone()
            };

            let transcript_info = build_transcript_info(&ep_desc).map(|mut info| {
                if let Some(w) = hf_warning { info.push_str(&format!("\n  ⚠ AI summary: {w}")); }
                info
            });

            return finish_add_learning(
                app, url, &title, hours, minutes, &date, &description, transcript_info,
            )
            .await;
        }

        println!("[Apple] Episode not found in feed — falling back to show-level metadata");
    }

    // ── Show-only path (no episode, or episode lookup failed) ────────────────
    let title = match (artist.as_str(), show_name.as_str()) {
        ("", name) => name.to_string(),
        (a, name) => format!("{a}: {name}"),
    };

    // iTunes API returns trackTimeMillis for episodes; shows have no duration.
    // We default to 0h 0m — the user can correct it in the YourLearning form.
    let duration_secs = show["trackTimeMillis"]
        .as_u64()
        .map(|ms| ms / 1000)
        .unwrap_or(0);
    let (hours, minutes) = split_duration(duration_secs);

    let show_desc = show["description"].as_str().unwrap_or("").to_string();

    let date = if !date_override.trim().is_empty() {
        NaiveDate::parse_from_str(date_override.trim(), "%Y-%m-%d")
            .map(|d| d.format("%Y/%m/%d").to_string())
            .unwrap_or_else(|_| date_override.trim().to_string())
    } else {
        Local::now().format("%Y/%m/%d").to_string()
    };

    let mut hf_warning: Option<String> = None;
    let description = if use_ai_summary && hf_api_token(app).is_some() && !show_desc.is_empty() {
        let summary_result = summarize_with_bart(app, &show_desc).await;
        println!("[Apple] Summary: {summary_result:?}");
        match summary_result {
            Ok(Some(s)) => s,
            Ok(None) => show_desc.clone(),
            Err(e) => { hf_warning = Some(e); show_desc.clone() }
        }
    } else {
        show_desc.clone()
    };

    let transcript_info = build_transcript_info(&show_desc).map(|mut info| {
        if let Some(w) = hf_warning { info.push_str(&format!("\n  ⚠ AI summary: {w}")); }
        info
    });

    finish_add_learning(app, url, &title, hours, minutes, &date, &description, transcript_info).await
}

/// Builds the analytics info line from a description/text blob.
/// Returns None if the text is empty.
fn build_transcript_info(text: &str) -> Option<String> {
    if text.trim().is_empty() {
        return None;
    }
    let (words, read_mins) = transcript_stats(text);
    let lix = compute_lix(text);
    Some(match lix {
        Some(score) => format!(
            "  Description: {} words  |  ~{} min read\n  LIX score:   {:.1} — {}",
            words, read_mins, score, lix_label(score)
        ),
        None => format!("  Description: {} words  |  ~{} min read", words, read_mins),
    })
}

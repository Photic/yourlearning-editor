use super::common::{build_description_analytics, finish_add_learning, hf_api_token, split_duration, summarize_with_bart};
use crate::http;
use chrono::{Local, NaiveDate};
use dioxus_logger::tracing;

// ── iTunes Lookup API ─────────────────────────────────────────────────────────

/// Extracts the numeric Apple Podcasts ID from a URL of the form
/// `https://podcasts.apple.com/…/idNNNNNNNNN` (show or episode page).
/// Also extracts the episode ID from the `?i=NNNNN` query parameter when present.
fn parse_apple_url(url: &str) -> Option<(String, Option<String>)> {
    // Separate the path from the query string first, since the "id…" segment
    // and the query params are not always separated by a '/' (e.g.
    // ".../id1500746737?i=1000783964365&l=da").
    let (path, query) = match url.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (url, None),
    };

    // Show/podcast ID: the path segment starting with "id"
    let podcast_id = path
        .split('/')
        .find(|seg| seg.starts_with("id") && seg.len() > 2 && seg[2..].chars().all(|c| c.is_ascii_digit()))?
        .trim_start_matches("id")
        .to_string();

    // Optional episode ID from `?i=...` query parameter
    let episode_id = query.and_then(|qs| {
        qs.split('&')
            .find(|p| p.starts_with("i="))
            .map(|p| p[2..].to_string())
    });

    Some((podcast_id, episode_id))
}

/// Calls the iTunes Lookup API for a podcast show or episode.
/// Returns the raw JSON `Value` of the first (and usually only) result.
async fn itunes_lookup(id: &str, entity: &str) -> Option<serde_json::Value> {
    let url = format!("https://itunes.apple.com/lookup?id={id}&entity={entity}&limit=1");
    let text = http::get(&url, &[("User-Agent", "Mozilla/5.0 (compatible; yourlearning-editor)")], 15_000)
        .await
        .ok()?;

    let json: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| tracing::debug!("[Apple] iTunes parse error: {e}"))
        .ok()?;

    // The API returns { "resultCount": N, "results": [...] }
    // Index 0 is the show itself (wrapperType=podcast), index 1 onward are episodes.
    // For an episode lookup we want the result with wrapperType == "podcastEpisode".
    let results = json["results"].as_array()?;
    results.first().cloned()
}

/// Fetches an Apple Podcasts episode page directly and extracts its embedded
/// `schema.org/PodcastEpisode` JSON-LD block.
///
/// This is the authoritative source for episode title/duration/date: unlike
/// the `i=` episode ID in the URL (which is an Apple-internal catalog ID with
/// no relationship to the podcast's own RSS feed — it does not appear in the
/// feed's `<guid>` or item URLs for most shows), the JSON-LD block is present
/// on the page itself and uses locale-independent ISO 8601 formats, so it
/// works regardless of the page's display language.
async fn fetch_episode_jsonld(url: &str) -> Option<(String, u64, String, String)> {
    let html = http::get(
        url,
        &[("User-Agent", "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")],
        15_000,
    )
    .await
    .ok()?;

    let json = extract_podcast_episode_jsonld(&html)?;

    let title = json["name"].as_str()?.to_string();
    let duration_secs = json["duration"].as_str().map(parse_iso8601_duration).unwrap_or(0);
    let pub_date = json["datePublished"]
        .as_str()
        .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
        .map(|d| d.format("%Y/%m/%d").to_string())
        .unwrap_or_default();
    let description = json["description"].as_str().unwrap_or("").to_string();

    Some((title, duration_secs, pub_date, description))
}

/// Scans `html` for `<script type="application/ld+json">` blocks and returns
/// the parsed JSON of the first one whose `@type` is `"PodcastEpisode"`.
fn extract_podcast_episode_jsonld(html: &str) -> Option<serde_json::Value> {
    let marker = "application/ld+json";
    let mut search_from = 0;

    while let Some(rel) = html[search_from..].find(marker) {
        let marker_abs = search_from + rel;
        let tag_close = html[marker_abs..].find('>')? + marker_abs + 1;
        let content_end = html[tag_close..].find("</script>")? + tag_close;
        let json_text = &html[tag_close..content_end];

        if let Ok(value) = serde_json::from_str::<serde_json::Value>(json_text) {
            if value["@type"].as_str() == Some("PodcastEpisode") {
                return Some(value);
            }
        }

        search_from = content_end + "</script>".len();
    }

    None
}

/// Parses a simple ISO 8601 duration of the form `PT#H#M#S` (all components
/// optional) into whole seconds, e.g. `"PT20M3S"` -> 1203.
fn parse_iso8601_duration(s: &str) -> u64 {
    let s = s.strip_prefix("PT").unwrap_or(s);
    let mut secs = 0u64;
    let mut num = String::new();

    for c in s.chars() {
        if c.is_ascii_digit() {
            num.push(c);
            continue;
        }
        let n: u64 = num.parse().unwrap_or(0);
        num.clear();
        match c {
            'H' => secs += n * 3600,
            'M' => secs += n * 60,
            'S' => secs += n,
            _ => {}
        }
    }

    secs
}

/// Fetches the podcast's RSS feed and returns the full `<description>` (or
/// `<itunes:summary>`) of the item whose `<title>` matches `title`
/// (case-insensitive, trimmed). Used to enrich the JSON-LD episode metadata
/// — which only carries a short, truncated description — with the full show
/// notes, since matching by title is far more reliable across feeds than
/// matching by Apple's episode ID (see `fetch_episode_jsonld`).
async fn find_description_in_feed(feed_url: &str, title: &str) -> Option<String> {
    let xml = http::get(feed_url, &[("User-Agent", "Mozilla/5.0 (compatible; yourlearning-editor)")], 30_000)
        .await
        .ok()?;

    parse_rss_description_by_title(&xml, title)
}

fn parse_rss_description_by_title(xml: &str, title: &str) -> Option<String> {
    let target = title.trim().to_lowercase();
    let lower = xml.to_lowercase();
    let mut pos = 0;

    while let Some(item_start) = lower[pos..].find("<item>").map(|i| pos + i) {
        let item_end = lower[item_start..].find("</item>").map(|i| item_start + i + 7)?;
        let item = &xml[item_start..item_end];

        if extract_xml_text(item, "title").is_some_and(|t| t.trim().to_lowercase() == target) {
            return extract_xml_text(item, "itunes:summary").or_else(|| extract_xml_text(item, "description"));
        }

        pos = item_end;
    }

    None
}

/// Looks up a specific episode by its episode ID using the podcast's feed URL.
/// The iTunes Search API does not support direct episode lookup by episode ID
/// reliably, so we fetch the RSS feed and find the episode by its GUID or by
/// matching the numeric episode ID embedded in the feed item URL.
///
/// This is a fallback for when `fetch_episode_jsonld` is unavailable — in
/// practice most feeds don't embed Apple's episode ID anywhere, so this
/// rarely matches, but it's cheap insurance.
async fn lookup_episode_from_feed(feed_url: &str, episode_id: &str) -> Option<(String, u64, String, String)> {
    // episode_id here is the numeric string from ?i=NNNNN
    let xml = http::get(feed_url, &[("User-Agent", "Mozilla/5.0 (compatible; yourlearning-editor)")], 30_000)
        .await
        .ok()?;

    // Walk every <item> and look for the one whose enclosure/guid contains the
    // episode_id string (Apple embeds the episode ID in the episode GUID or URL).
    parse_rss_episode_by_id(&xml, episode_id)
}

/// Parses an RSS feed XML string and finds the episode whose `<guid>` or any
/// enclosure URL contains `episode_id`.  Returns (title, duration_secs, pub_date, description).
fn parse_rss_episode_by_id(xml: &str, episode_id: &str) -> Option<(String, u64, String, String)> {
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
    chrono::DateTime::parse_from_rfc2822(s.trim())
        .map(|dt| dt.format("%Y/%m/%d").to_string())
        .unwrap_or_else(|_| Local::now().format("%Y/%m/%d").to_string())
}

// ── Public handler ────────────────────────────────────────────────────────────

/// Handles an Apple Podcasts URL:
/// - Looks up show + optional episode via the iTunes API.
/// - If an episode is specified, tries to find a description from the RSS feed.
/// - Optionally summarises the description with BART.
pub(crate) async fn run_apple_podcast(url: &str, date_override: &str, use_ai_summary: bool) -> Result<(), String> {
    tracing::debug!("[Apple] Fetching metadata for {url}");

    let (podcast_id, episode_id) = parse_apple_url(url)
        .ok_or_else(|| "Could not extract podcast ID from the Apple Podcasts URL.".to_string())?;

    tracing::debug!("[Apple] Podcast ID: {podcast_id}, Episode ID: {episode_id:?}");

    // Always look up the show first to get the feed URL and show name.
    let show = itunes_lookup(&podcast_id, "podcast")
        .await
        .ok_or_else(|| "iTunes API returned no results for this podcast.".to_string())?;

    let show_name = show["collectionName"].as_str().unwrap_or("").to_string();
    let artist = show["artistName"].as_str().unwrap_or("").to_string();
    let feed_url = show["feedUrl"].as_str().unwrap_or("").to_string();

    tracing::debug!("[Apple] Show: {show_name:?}, Artist: {artist:?}");

    // ── Episode path ─────────────────────────────────────────────────────────
    if let Some(ref ep_id) = episode_id {
        tracing::debug!("[Apple] Looking up episode {ep_id} via page JSON-LD");

        let episode = if let Some((ep_title, duration_secs, pub_date_str, short_desc)) = fetch_episode_jsonld(url).await {
            let full_desc = if !feed_url.is_empty() {
                find_description_in_feed(&feed_url, &ep_title).await
            } else {
                None
            };
            Some((ep_title, duration_secs, pub_date_str, full_desc.unwrap_or(short_desc)))
        } else if !feed_url.is_empty() {
            tracing::debug!("[Apple] JSON-LD unavailable — falling back to feed ID match");
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
            let description = if use_ai_summary && hf_api_token().await.is_some() && !ep_desc.is_empty() {
                let summary_result = summarize_with_bart(&ep_desc).await;
                tracing::debug!("[Apple] Summary: {summary_result:?}");
                match summary_result {
                    Ok(Some(s)) => s,
                    Ok(None) => ep_desc.clone(),
                    Err(e) => { hf_warning = Some(e); ep_desc.clone() }
                }
            } else {
                ep_desc.clone()
            };

            let analytics = build_description_analytics(&ep_desc, hf_warning);

            return finish_add_learning(url, &title, hours, minutes, &date, &description, analytics).await;
        }

        tracing::debug!("[Apple] Episode not found in feed — falling back to show-level metadata");
    }

    // ── Show-only path (no episode, or episode lookup failed) ────────────────
    let title = match (artist.as_str(), show_name.as_str()) {
        ("", name) => name.to_string(),
        (a, name) => format!("{a}: {name}"),
    };

    // iTunes API returns trackTimeMillis for episodes; shows have no duration.
    // We default to 0h 0m — the user can correct it in the YourLearning form.
    let duration_secs = show["trackTimeMillis"].as_u64().map(|ms| ms / 1000).unwrap_or(0);
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
    let description = if use_ai_summary && hf_api_token().await.is_some() && !show_desc.is_empty() {
        let summary_result = summarize_with_bart(&show_desc).await;
        tracing::debug!("[Apple] Summary: {summary_result:?}");
        match summary_result {
            Ok(Some(s)) => s,
            Ok(None) => show_desc.clone(),
            Err(e) => { hf_warning = Some(e); show_desc.clone() }
        }
    } else {
        show_desc.clone()
    };

    let analytics = build_description_analytics(&show_desc, hf_warning);

    finish_add_learning(url, &title, hours, minutes, &date, &description, analytics).await
}

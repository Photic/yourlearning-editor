use super::common::{build_description_analytics, finish_add_learning, hf_api_token, split_duration, summarize_with_bart};
use crate::http;
use chrono::{Local, NaiveDate};
use dioxus_logger::tracing;

// ── RSS feed fetch ────────────────────────────────────────────────────────────

async fn fetch_feed(url: &str) -> Result<String, String> {
    http::get(url, &[("User-Agent", "Mozilla/5.0 (compatible; yourlearning-editor)")], 30_000)
        .await
        .map_err(|e| format!("Failed to fetch RSS feed: {e}"))
}

// ── XML helpers ───────────────────────────────────────────────────────────────

/// Returns the text content of the first `<tag>…</tag>` or `<ns:tag>…</ns:tag>`
/// pair (case-insensitive).  Handles CDATA sections.
fn extract_xml_text(xml: &str, tag: &str) -> Option<String> {
    let lower = xml.to_lowercase();
    let tag_lower = tag.to_lowercase();

    // Try exact open tag first, then with attributes
    let open_exact = format!("<{tag_lower}>");
    let open_attr = format!("<{tag_lower} ");

    let tag_start = lower.find(&open_exact).or_else(|| lower.find(&open_attr))?;
    let content_start = xml[tag_start..].find('>')? + tag_start + 1;
    let close = format!("</{tag_lower}>");
    let content_end = lower[content_start..].find(&close)? + content_start;

    let raw = xml[content_start..content_end].trim();

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

// ── RSS channel + episode parsing ────────────────────────────────────────────

struct ChannelMeta {
    title: String,
    author: String,
}

/// Extracts the show-level metadata from the <channel> block (before any <item>).
fn parse_channel_meta(xml: &str) -> ChannelMeta {
    // Scope the search to everything before the first <item> so we don't
    // accidentally pick up episode-level tags with the same name.
    let lower = xml.to_lowercase();
    let channel_end = lower.find("<item").unwrap_or(xml.len());
    let channel_xml = &xml[..channel_end];

    let title = extract_xml_text(channel_xml, "title").unwrap_or_default();
    let author = extract_xml_text(channel_xml, "itunes:author")
        .or_else(|| extract_xml_text(channel_xml, "author"))
        .unwrap_or_default();

    ChannelMeta { title, author }
}

struct EpisodeMeta {
    title: String,
    duration_secs: u64,
    pub_date: String,
    description: String,
}

/// Parses the first `<item>` block found in the feed (the latest episode).
fn parse_latest_episode(xml: &str) -> Option<EpisodeMeta> {
    let lower = xml.to_lowercase();
    let item_start = lower.find("<item>")?;
    let item_end = lower[item_start..].find("</item>").map(|i| item_start + i + 7)?;
    let item = &xml[item_start..item_end];

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

    Some(EpisodeMeta { title, duration_secs, pub_date, description })
}

// ── Public handler ────────────────────────────────────────────────────────────

/// Handles a raw RSS podcast feed URL.
///
/// Fetches the feed, extracts the show name and the latest episode, then
/// optionally summarises the episode description with BART.  If anything
/// cannot be extracted the field is left empty so the user can fill it in
/// manually on the YourLearning form.
pub(crate) async fn run_rss_podcast(url: &str, date_override: &str, use_ai_summary: bool) -> Result<(), String> {
    tracing::debug!("[RSS] Fetching feed {url}");
    let xml = fetch_feed(url).await?;

    let channel = parse_channel_meta(&xml);
    tracing::debug!("[RSS] Channel: {:?} by {:?}", channel.title, channel.author);

    let episode = parse_latest_episode(&xml);

    let (title, duration_secs, pub_date_str, text_for_summary) = match episode {
        Some(ep) => {
            tracing::debug!("[RSS] Episode: {:?} ({} secs)", ep.title, ep.duration_secs);
            let title = if channel.title.is_empty() {
                ep.title
            } else if ep.title.is_empty() {
                channel.title.clone()
            } else {
                format!("{}: {}", channel.title, ep.title)
            };
            (title, ep.duration_secs, ep.pub_date, ep.description)
        }
        None => {
            tracing::debug!("[RSS] No episodes found — using channel-level metadata only");
            let title = match (channel.author.as_str(), channel.title.as_str()) {
                ("", name) => name.to_string(),
                (a, name) => format!("{a}: {name}"),
            };
            (title, 0u64, String::new(), String::new())
        }
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

    // ── Optionally summarise ──────────────────────────────────────────────────
    let mut hf_warning: Option<String> = None;
    let description = if use_ai_summary && hf_api_token().await.is_some() && !text_for_summary.is_empty() {
        let summary_result = summarize_with_bart(&text_for_summary).await;
        tracing::debug!("[RSS] Summary: {summary_result:?}");
        match summary_result {
            Ok(Some(s)) => s,
            Ok(None) => text_for_summary.clone(),
            Err(e) => { hf_warning = Some(e); text_for_summary.clone() }
        }
    } else {
        text_for_summary.clone()
    };

    let analytics = build_description_analytics(&text_for_summary, hf_warning);

    finish_add_learning(url, &title, hours, minutes, &date, &description, analytics).await
}

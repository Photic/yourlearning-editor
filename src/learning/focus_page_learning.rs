use super::common::{compute_lix, finish_add_learning, hf_api_token, lix_label, summarize_with_bart, transcript_stats};
use crate::{browser, storage};
use chrono::{Local, NaiveDate};
use dioxus_logger::tracing;

/// Reads the browser's currently active tab and interprets its rendered
/// text as a learning entry — the counterpart to the other handlers in this
/// module, except the source is whatever page the user is already looking
/// at rather than a URL they paste in.
///
/// Nothing here runs until the user explicitly asks for it (clicking "Add
/// Page Learning" in the popup) — the DOM is only ever read on that click,
/// never observed passively.
pub async fn run_focus_page_learning(date_override: &str, use_ai_summary: bool) -> Result<(), String> {
    let (tab_id, url) = browser::active_tab().await?;

    if url.is_empty() || is_restricted_url(&url) {
        return Err(
            "Can't read this page — browser and extension pages aren't accessible to the extension.".to_string(),
        );
    }

    tracing::debug!("[FocusPage] Reading DOM of {url}");
    let (dom_title, dom_text) = browser::read_page_dom(tab_id).await?;

    let title = if dom_title.trim().is_empty() { url.clone() } else { dom_title.trim().to_string() };
    let body_text = dom_text.trim().to_string();

    if body_text.is_empty() {
        return Err("Could not find any readable text on this page.".to_string());
    }

    tracing::debug!("[FocusPage] Title: {title:?}");
    tracing::debug!("[FocusPage] Body text ({} chars)", body_text.len());

    // ── Compute LIX / reading time (always, no token needed) ─────────────────
    let lix = compute_lix(&body_text);
    let (words, _) = transcript_stats(&body_text);

    // Duration for YourLearning = estimated reading time adjusted for difficulty,
    // same banding as the article handler.
    let wpm = match lix.unwrap_or(35.0) as u32 {
        0..=24  => 150, // Very easy  — lower bound ~150 wpm
        25..=34 => 120, // Easy       — lower bound ~120 wpm
        35..=44 =>  90, // Medium     — lower bound  ~90 wpm
        45..=54 =>  70, // Difficult  — lower bound  ~70 wpm
        _       =>  50, // Very difficult — lower bound ~50 wpm
    };
    let total_read_secs = (words as u64 * 60 + (wpm - 1)) / wpm; // ceiling at wpm
    let total_read_mins = (total_read_secs + 59) / 60;           // ceiling to whole minutes
    let total_read_mins = total_read_mins.max(1);                 // at least 1 min
    let (hours, minutes) = (total_read_mins / 60, total_read_mins % 60);

    // ── Optionally summarise ─────────────────────────────────────────────────
    // The caller (the popup) is responsible for warning the user that the
    // page's content is about to be sent to a third-party AI service before
    // it ever invokes this with `use_ai_summary: true` — this is just the
    // send.
    let mut hf_warning: Option<String> = None;
    let description = if use_ai_summary && hf_api_token().await.is_some() {
        let summary_result = summarize_with_bart(&body_text).await;
        tracing::debug!("[FocusPage] Summary: {summary_result:?}");
        match summary_result {
            Ok(Some(s)) => s,
            Ok(None) => String::new(),
            Err(e) => { hf_warning = Some(e); String::new() }
        }
    } else {
        String::new()
    };

    // ── Analytics ────────────────────────────────────────────────────────────
    let display_mins = total_read_mins;
    let analytics = Some(storage::AnalyticsInfo {
        primary_label: "Page".to_string(),
        primary_value: format!("{words} words  |  ~{display_mins} min read (@ {wpm}wpm)"),
        lix: lix.map(|score| storage::LixScore { score, label: lix_label(score).to_string() }),
        warning: hf_warning,
    });

    // ── Date ─────────────────────────────────────────────────────────────────
    let today = if !date_override.trim().is_empty() {
        NaiveDate::parse_from_str(date_override.trim(), "%Y-%m-%d")
            .map(|d| d.format("%Y/%m/%d").to_string())
            .unwrap_or_else(|_| date_override.trim().to_string())
    } else {
        Local::now().format("%Y/%m/%d").to_string()
    };

    finish_add_learning(&url, &title, hours, minutes, &today, &description, analytics).await
}

/// True for browser/extension-internal pages that `chrome.scripting` can
/// never inject into (chrome://, extension pages, the Web Store, etc.) —
/// checked up front so the failure is a clear message instead of an opaque
/// API rejection.
fn is_restricted_url(url: &str) -> bool {
    let lower = url.to_lowercase();
    lower.starts_with("chrome://")
        || lower.starts_with("chrome-extension://")
        || lower.starts_with("edge://")
        || lower.starts_with("about:")
        || lower.starts_with("https://chrome.google.com/webstore")
        || lower.starts_with("https://chromewebstore.google.com")
}

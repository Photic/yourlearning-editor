use super::common::{
    compute_lix, extract_article, finish_add_learning, hf_api_token, is_self_fetching_media_url, lix_label,
    summarize_with_bart, transcript_stats, ArticleDoc,
};
use crate::{browser, http, storage};
use chrono::{Local, NaiveDate};
use dioxus_logger::tracing;

// ── HTTP helpers ──────────────────────────────────────────────────────────────

/// Fetches the raw markup of `url` and reads the article out of it.
///
/// This is the cheap first attempt for a pasted URL: one request, no
/// rendering. It gets the whole article from any server-rendered page, which
/// is most of them. What it can't do is run the page's JS — so an SPA hands
/// back an empty shell, and a site that fingerprints clients hands back a bot
/// check. Both fail the extraction's word floor rather than passing off
/// boilerplate as content, which is what sends the caller to a real tab.
async fn fetch_and_extract(url: &str) -> Result<ArticleDoc, String> {
    let html = http::get(url, &[("Accept", "text/html,application/xhtml+xml")], 30_000)
        .await
        .map_err(|e| format!("Fetching the page failed: {e}"))?;
    tracing::debug!("[Article] Fetched {} chars of markup", html.len());
    extract_article(&html, url)
}

/// Falls back to the page's rendered text when the readability pass came up
/// empty. `text` is `document.body.innerText` — everything a sighted reader
/// would see, nav and footer included — so it's a blunter instrument than a
/// real extraction, and the returned warning says so. Still better than
/// failing outright when we're holding the content already.
fn article_from_rendered_text(page: &browser::PageDom, url: &str, reason: &str) -> Result<(ArticleDoc, String), String> {
    let body = page.text.trim().to_string();
    let words = body.split_whitespace().count();
    if words < 50 {
        return Err(reason.to_string());
    }

    let title = if page.title.trim().is_empty() { url.to_string() } else { page.title.trim().to_string() };
    Ok((
        ArticleDoc { title, body, published: None },
        format!("{reason} Used the page's visible text instead, so the word count includes navigation and footers."),
    ))
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

// ── Public handlers ──────────────────────────────────────────────────────────

/// Handles any non-YouTube/podcast/RSS/Vimeo URL — the "Add Learning" tab's
/// paste-a-URL path, where the user isn't already sitting on the page.
/// Fetches it, reads the article out of the markup, optionally summarises
/// with BART, computes LIX, then hands off to YourLearning.
///
/// A plain fetch can't run a page's JS, and some sites won't serve one at all
/// without a browser to look at. When that first attempt yields no article,
/// the URL is loaded in a background browser tab instead and read from its
/// rendered DOM — a real tab runs the page's scripts and clears the bot
/// checks a bare request can't, so it succeeds where the fetch didn't.
pub(crate) async fn run_article(url: &str, date_override: &str, use_ai_summary: bool) -> Result<(), String> {
    tracing::debug!("[Article] Fetching {url}");
    let (doc, warning) = match fetch_and_extract(url).await {
        Ok(doc) => (doc, None),
        Err(fetch_err) => {
            tracing::debug!("[Article] Plain fetch yielded no article ({fetch_err}) — loading it in a background tab");
            let page = browser::read_url_in_background_tab(url)
                .await
                .map_err(|e| format!("Could not extract this page's content: {fetch_err}; loading it in a tab also failed: {e}"))?;

            match extract_article(&page.html, url) {
                Ok(doc) => (doc, None),
                Err(dom_err) => {
                    let (doc, warning) = article_from_rendered_text(&page, url, &dom_err)
                        .map_err(|e| format!("Could not extract this page's content: {e}"))?;
                    (doc, Some(warning))
                }
            }
        }
    };

    let title = if doc.title.trim().is_empty() { url.to_string() } else { doc.title.trim().to_string() };
    finish_learning_entry(url, "Article", &title, &doc.body, doc.published, date_override, use_ai_summary, warning)
        .await
}

/// Reads the browser's currently active tab and interprets its rendered
/// text as a learning entry — the counterpart to `run_article`, except the
/// source is whatever page the user is already looking at rather than a URL
/// they paste in.
///
/// Nothing here runs until the user explicitly asks for it (clicking "Add
/// Page Learning" in the popup) — the DOM is only ever read on that click,
/// never observed passively.
pub(crate) async fn run_focus_page(date_override: &str, use_ai_summary: bool) -> Result<(), String> {
    let (tab_id, url) = browser::active_tab().await?;

    if url.is_empty() || is_restricted_url(&url) {
        return Err(
            "Can't read this page — browser and extension pages aren't accessible to the extension.".to_string(),
        );
    }

    // YouTube/podcast/RSS/Vimeo pull structured metadata from their own APIs
    // and feeds, so there's nothing the DOM could add — hand those straight
    // to `run_add_learning`. Articles deliberately fall through to the DOM
    // read below even when recognized: this tab has already rendered the page
    // and cleared any bot check, which a server-side fetch of the same URL
    // can't be relied on to do.
    if is_self_fetching_media_url(&url) {
        tracing::debug!("[FocusPage] Self-fetching media path — routing directly");
        return super::run_add_learning(&url, date_override, use_ai_summary).await;
    }

    tracing::debug!("[FocusPage] Capturing DOM of {url}");
    let page = browser::read_page_dom(tab_id).await?;

    // The captured HTML goes through the readability pass rather than being
    // picked apart here: page structure varies too much between sites to
    // guess at, and `innerText` alone can't tell nav/ads/boilerplate from the
    // article itself. The pass runs locally, so the page never leaves the
    // browser — this tab has already rendered it, which is the one thing no
    // outside fetch of the same URL could reproduce.
    let (doc, warning) = match extract_article(&page.html, &url) {
        Ok(doc) => (doc, None),
        Err(dom_err) => {
            let (doc, warning) = article_from_rendered_text(&page, &url, &dom_err)
                .map_err(|e| format!("Could not extract this page's content: {e}"))?;
            (doc, Some(warning))
        }
    };

    let title = if doc.title.trim().is_empty() { url.clone() } else { doc.title.trim().to_string() };
    tracing::debug!("[FocusPage] Extracted {} words", doc.body.split_whitespace().count());

    finish_learning_entry(&url, "Page", &title, &doc.body, doc.published, date_override, use_ai_summary, warning).await
}

/// Shared tail end of both handlers above: computes LIX/reading time,
/// optionally summarises with BART, builds the analytics block, and submits
/// to YourLearning. `extraction_warning` carries a diagnostic from whichever
/// readability step ran upstream (a page that yielded no article, thin
/// content, etc.) so it can be surfaced next to any summarisation warning
/// instead of being silently dropped.
///
/// `published` is the article's own publication date as Jina reported it,
/// used when the user didn't set a date themselves.
async fn finish_learning_entry(
    url: &str,
    primary_label: &str,
    title: &str,
    body_text: &str,
    published: Option<String>,
    date_override: &str,
    use_ai_summary: bool,
    extraction_warning: Option<String>,
) -> Result<(), String> {
    tracing::debug!("[{primary_label}] Title: {title:?}");
    tracing::debug!("[{primary_label}] Body text ({} chars)", body_text.len());

    // ── Compute LIX / reading time (always, no token needed) ─────────────────
    let lix = compute_lix(body_text);
    let (words, _) = transcript_stats(body_text);

    // Duration for YourLearning = estimated reading time adjusted for difficulty.
    // Conservative: use the lower bound of each band's expected reading speed.
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
    let mut hf_warning: Option<String> = None;
    let description = if use_ai_summary && hf_api_token().await.is_some() {
        let summary_result = summarize_with_bart(body_text).await;
        tracing::debug!("[{primary_label}] Summary: {summary_result:?}");
        match summary_result {
            Ok(Some(s)) => s,
            Ok(None) => String::new(),
            Err(e) => { hf_warning = Some(e); String::new() }
        }
    } else {
        String::new()
    };

    // ── Analytics ────────────────────────────────────────────────────────────
    // Use the same adjusted reading time that was sent to YourLearning.
    let display_mins = total_read_mins;
    let warning = match (extraction_warning, hf_warning) {
        (Some(e), Some(h)) => Some(format!("{e}; {h}")),
        (Some(e), None) => Some(e),
        (None, Some(h)) => Some(h),
        (None, None) => None,
    };
    let analytics = Some(storage::AnalyticsInfo {
        primary_label: primary_label.to_string(),
        primary_value: format!("{words} words  |  ~{display_mins} min read (@ {wpm}wpm)"),
        lix: lix.map(|score| storage::LixScore { score, label: lix_label(score).to_string() }),
        warning,
    });

    // ── Date ─────────────────────────────────────────────────────────────────
    // A date the user typed always wins; otherwise use the article's own
    // publication date, falling back to today only when the page didn't
    // report one.
    let today = if !date_override.trim().is_empty() {
        NaiveDate::parse_from_str(date_override.trim(), "%Y-%m-%d")
            .map(|d| d.format("%Y/%m/%d").to_string())
            .unwrap_or_else(|_| date_override.trim().to_string())
    } else if let Some(published) = published {
        tracing::debug!("[{primary_label}] Using published date {published}");
        published
    } else {
        Local::now().format("%Y/%m/%d").to_string()
    };

    finish_add_learning(url, title, hours, minutes, &today, &description, analytics).await
}

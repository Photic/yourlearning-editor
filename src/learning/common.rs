use super::{apple_podcast, article, rss_podcast, spotify_podcast, vimeo, youtube};
use crate::browser::sleep;
use crate::{browser, http, storage};
use dioxus_logger::tracing;
use dom_smoothie::{Article, Config, Readability};

pub(crate) const YOURLEARNING_URL: &str = "https://yourlearning.ibm.com/add-learning";
const PENDING_ADD_LEARNING_KEY: &str = "PENDING_ADD_LEARNING";

// ── URL routing ───────────────────────────────────────────────────────────────

/// Detects which kind of learning a URL represents and dispatches to the
/// appropriate handler.  This is the single entry point the UI calls for
/// adding a learning — all URL routing lives here so individual handler
/// modules stay focused on their own media type.
pub async fn run_add_learning(url: &str, date_override: &str, use_ai_summary: bool) -> Result<(), String> {
    let url = url.trim().to_string();

    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(
            "Please paste a full URL starting with https://\n\nSupported sources:\n  • YouTube       youtube.com/watch or youtu.be\n  • Apple Podcast podcasts.apple.com\n  • Spotify       open.spotify.com/episode\n  • Vimeo         vimeo.com\n  • RSS feed      .xml / .rss or known feed hosts\n  • Article       any other https:// page".to_string()
        );
    }

    if is_youtube_url(&url) {
        tracing::debug!("Youtube Entry");
        youtube::run_youtube_learning(&url, date_override, use_ai_summary).await
    } else if url.contains("podcasts.apple.com") {
        tracing::debug!("Apple Podcast Entry");
        apple_podcast::run_apple_podcast(&url, date_override, use_ai_summary).await
    } else if url.contains("open.spotify.com/episode/") {
        tracing::debug!("Spotify Podcast Entry");
        spotify_podcast::run_spotify_podcast(&url, date_override, use_ai_summary).await
    } else if is_rss_feed_url(&url) {
        tracing::debug!("RSS Podcast Entry");
        rss_podcast::run_rss_podcast(&url, date_override, use_ai_summary).await
    } else if is_vimeo_url(&url) {
        tracing::debug!("Vimeo Entry");
        vimeo::run_vimeo(&url, date_override, use_ai_summary).await
    } else {
        tracing::debug!("Default (Article) Entry");
        article::run_article(&url, date_override, use_ai_summary).await
    }
}

/// Reads the browser's currently active tab and interprets its rendered
/// text as a learning entry. Unlike `run_add_learning`, there's no URL to
/// route on — the source is whatever page the user is already looking at.
pub async fn run_focus_page_learning(date_override: &str, use_ai_summary: bool) -> Result<(), String> {
    article::run_focus_page(date_override, use_ai_summary).await
}

/// Returns true for YouTube watch pages (both the standard `youtube.com/watch`
/// URL and the `youtu.be/` short-link form) — shared between `run_add_learning`
/// and the focus-page handler, which uses it to route straight to the YouTube
/// path instead of reading the page DOM.
pub(crate) fn is_youtube_url(url: &str) -> bool {
    url.contains("youtube.com/watch") || url.contains("youtu.be/")
}

/// Returns true if `url` matches one of the learning paths above that fetch
/// their own metadata directly (YouTube, Apple Podcasts, Spotify, RSS,
/// Vimeo, known article publishers) rather than needing the page's rendered
/// DOM.
///
/// There's no reliable way to detect from a URL alone whether a page sits
/// behind a login or is otherwise sensitive — a content script can't inspect
/// auth state or document classification before rendering it. So rather than
/// try to flag "sensitive" pages, this flips the check around: only pages we
/// already know how to read *without* touching their DOM get to skip the
/// focus-page consent prompt. Everything else — including pages we've simply
/// never seen before — still asks.
pub fn is_known_learning_url(url: &str) -> bool {
    is_self_fetching_media_url(url) || is_known_article_url(url)
}

/// Returns true for the sources whose handlers pull structured metadata from
/// a dedicated API or feed (YouTube, Apple Podcasts, Spotify, RSS, Vimeo) —
/// i.e. the ones that genuinely have nothing to gain from the page's DOM.
///
/// Article publishers are deliberately *not* included: recognizing one is
/// only reason enough to skip the consent prompt, never reason to throw away
/// a rendered DOM we already have. When the user is looking at an article, a
/// real browser tab has already run its JS and cleared any bot check, so its
/// DOM beats anything a server-side fetch of the same URL can retrieve.
pub(crate) fn is_self_fetching_media_url(url: &str) -> bool {
    is_youtube_url(url)
        || url.contains("podcasts.apple.com")
        || url.contains("open.spotify.com/episode/")
        || is_rss_feed_url(url)
        || is_vimeo_url(url)
}

/// Returns true for a curated set of well-known article/blog/news publishers
/// — public pages whose content carries no expectation of privacy, so
/// `is_known_learning_url` lets them skip the focus-page consent prompt the
/// same way YouTube, podcasts, RSS feeds, and Vimeo do. Deliberately kept to
/// major, unambiguous publishers — obscure or personal blogs still ask first.
///
/// This governs *only* the prompt. Matching here doesn't change how the page
/// is read: articles go through the DOM-reading path either way.
fn is_known_article_url(url: &str) -> bool {
    let lower = url.to_lowercase();
    let article_domains = [
        // Publishing platforms / reference
        "medium.com",
        "substack.com",
        "dev.to",
        "hashnode.com",
        "wikipedia.org",
        "britannica.com",
        // General news
        "nytimes.com",
        "washingtonpost.com",
        "theguardian.com",
        "bbc.com",
        "bbc.co.uk",
        "reuters.com",
        "bloomberg.com",
        "wsj.com",
        "ft.com",
        "cnn.com",
        "npr.org",
        "apnews.com",
        "economist.com",
        "time.com",
        "usatoday.com",
        "latimes.com",
        "politico.com",
        "axios.com",
        "vox.com",
        "slate.com",
        "theatlantic.com",
        "newyorker.com",
        "nbcnews.com",
        "cbsnews.com",
        "abcnews.go.com",
        "aljazeera.com",
        "newsweek.com",
        // Business
        "forbes.com",
        "fortune.com",
        "hbr.org",
        "fastcompany.com",
        "businessinsider.com",
        "cnbc.com",
        "inc.com",
        "entrepreneur.com",
        // Tech news
        "wired.com",
        "techcrunch.com",
        "arstechnica.com",
        "theverge.com",
        "engadget.com",
        "venturebeat.com",
        "gizmodo.com",
        "zdnet.com",
        "cnet.com",
        "thenextweb.com",
        "techradar.com",
        "tomshardware.com",
        "mashable.com",
        // Science
        "nature.com",
        "scientificamerican.com",
        "newscientist.com",
        "phys.org",
        "quantamagazine.org",
        "sciencedaily.com",
        "spectrum.ieee.org",
        // Developer / engineering blogs
        "smashingmagazine.com",
        "css-tricks.com",
        "freecodecamp.org",
        "infoq.com",
        "stackoverflow.blog",
        "martinfowler.com",
        "realpython.com",
        // Company engineering / newsroom blogs
        "github.blog",
        "netflixtechblog.com",
        "engineering.fb.com",
        "developer.ibm.com",
        "ibm.com/blog",
        "aws.amazon.com/blogs",
        "blogs.microsoft.com",
        "news.microsoft.com",
        "techcommunity.microsoft.com",
        "blog.google",
        "developers.googleblog.com",
        "cloud.google.com/blog",
        "engineering.atspotify.com",
        "shopify.engineering",
        "slack.engineering",
        "eng.uber.com",
    ];
    article_domains.iter().any(|d| lower.contains(d))
}

/// Returns true if the URL looks like a direct podcast RSS feed rather than a
/// web page.  Heuristics (in priority order):
/// - well-known feed hosting domains
/// - common feed path segments (/feed, /rss, …)
/// - explicit .xml / .rss file extension
fn is_rss_feed_url(url: &str) -> bool {
    let lower = url.to_lowercase();

    // Well-known RSS hosting domains
    let feed_domains = [
        "feeds.simplecast.com",
        "feeds.buzzsprout.com",
        "feeds.transistor.fm",
        "feeds.soundcloud.com",
        "feeds.libsyn.com",
        "feeds.megaphone.fm",
        "feeds.acast.com",
        "feeds.captivate.fm",
        "feeds.podcastmirror.com",
        "anchor.fm/s/",
        "audioboom.com/channels/",
        "rss.art19.com",
        "omny.fm/shows/",
        "pinecast.com/feed/",
        "podcasts.files.bbci.co.uk",
    ];
    if feed_domains.iter().any(|d| lower.contains(d)) {
        return true;
    }

    // Path-segment heuristics
    let feed_segments = ["/feed/", "/feed.xml", "/rss", "/podcast.xml", "/episodes.xml"];
    if feed_segments.iter().any(|s| lower.contains(s)) {
        return true;
    }

    // Explicit .xml / .rss extension (strip query string first)
    let path = lower.split('?').next().unwrap_or(&lower);
    path.ends_with(".xml") || path.ends_with(".rss")
}

/// Returns true for vimeo.com watch pages and player.vimeo.com embed URLs.
fn is_vimeo_url(url: &str) -> bool {
    let lower = url.to_lowercase();
    lower.contains("vimeo.com/")
        && !lower.contains("vimeo.com/channels")
        && !lower.contains("vimeo.com/groups")
        && !lower.contains("vimeo.com/album")
}

// ── HF Inference API (bart-large-cnn) ────────────────────────────────────────

/// POSTs to `url` once and gives up on the first transport failure. The
/// summary is a nice-to-have on an entry that gets created either way, so a
/// slow or unreachable HF is worth reporting straight away rather than
/// retrying while the user sits waiting on it.
async fn post_json_once(
    url: &str,
    headers: &[(&str, &str)],
    body: &serde_json::Value,
    timeout_ms: u32,
) -> Result<String, String> {
    http::post_json(url, headers, body, timeout_ms).await.map_err(|e| {
        tracing::debug!("[HF] Request failed: {e}");
        // Phrased for the history entry this ends up attached to: the entry
        // itself was created, and only the description is missing — say so,
        // rather than leaving the reader to wonder what survived.
        format!(
            "AI summary skipped — HuggingFace didn't respond within {}s. The entry was saved without a description.",
            timeout_ms / 1_000
        )
    })
}

/// Returns the HuggingFace API token stored in extension settings.
pub(crate) async fn hf_api_token() -> Option<String> {
    storage::get_setting("HF_API_TOKEN")
        .await
        .ok()
        .flatten()
        .filter(|value| !value.is_empty())
}

/// Calls the Hugging Face Inference API to summarise `text` using
/// facebook/bart-large-cnn.
///
/// Returns:
/// - `Ok(None)`       — no HF token configured, summary skipped.
/// - `Ok(Some(text))` — summary produced successfully.
/// - `Err(msg)`       — token present but the request failed (network error,
///                      timeout, unexpected response).  `msg` is a short
///                      human-readable description suitable for display.
///
/// Handles the HF cold-start case: if the model is still loading the API
/// returns `{"error":"Loading…","estimated_time":<secs>}`.  We honour that
/// delay and retry once with `wait_for_model: true`.
pub(crate) async fn summarize_with_bart(text: &str) -> Result<Option<String>, String> {
    let token = match hf_api_token().await {
        Some(t) => t,
        None => {
            tracing::debug!("[HF] HF_API_TOKEN not set — skipping summary.");
            return Ok(None);
        }
    };

    // bart-large-cnn has a 1 024-token input limit; truncate conservatively.
    let input: String = text.chars().take(3000).collect();
    let auth_header = format!("Bearer {token}");

    // First attempt — fast path (model already warm).
    let raw = post_json_once(
        "https://router.huggingface.co/hf-inference/models/facebook/bart-large-cnn",
        &[("Authorization", auth_header.as_str())],
        &serde_json::json!({ "inputs": input }),
        15_000,
    )
    .await?;

    tracing::debug!("[HF] Response: {}", &raw[..raw.len().min(200)]);

    let value: serde_json::Value =
        serde_json::from_str(&raw).map_err(|e| format!("HF returned unexpected response: {e}"))?;

    // Model still loading? Wait the suggested delay then retry with
    // wait_for_model so the server blocks until it's ready.
    if value.get("error").is_some() {
        let wait_secs = value["estimated_time"].as_f64().unwrap_or(20.0);
        tracing::debug!("[HF] Model loading — waiting {wait_secs:.0}s then retrying…");
        sleep((wait_secs.min(60.0) * 1000.0) as i32).await;

        let raw2 = post_json_once(
            "https://router.huggingface.co/hf-inference/models/facebook/bart-large-cnn",
            &[
                ("Authorization", auth_header.as_str()),
                ("X-Wait-For-Model", "true"),
            ],
            &serde_json::json!({ "inputs": input }),
            60_000,
        )
        .await?;

        tracing::debug!("[HF] Retry response: {}", &raw2[..raw2.len().min(200)]);

        let value2: serde_json::Value = serde_json::from_str(&raw2)
            .map_err(|e| format!("HF retry returned unexpected response: {e}"))?;
        return Ok(value2
            .get(0)
            .and_then(|v| v.get("summary_text"))
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string()));
    }

    // Successful response: [{"summary_text": "..."}]
    Ok(value
        .get(0)
        .and_then(|v| v.get("summary_text"))
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string()))
}

// ── Transcript analytics ──────────────────────────────────────────────────────

/// Computes the LIX (Läsbarhetsindex) readability score for `text`.
///
/// Formula:  LIX = (words / sentences) + ((long_words * 100) / words)
/// where a "long word" is any word with more than 6 characters.
///
/// Returns `None` if the text has no words or no sentence-ending punctuation.
pub(crate) fn compute_lix(text: &str) -> Option<f64> {
    let words: Vec<&str> = text.split_whitespace().collect();
    let word_count = words.len();
    if word_count == 0 {
        return None;
    }

    let long_word_count = words
        .iter()
        .filter(|w| w.chars().filter(|c| c.is_alphabetic()).count() > 6)
        .count();

    // Count sentences by sentence-ending punctuation (.  !  ?)
    let sentence_count = text
        .chars()
        .filter(|&c| c == '.' || c == '!' || c == '?')
        .count()
        .max(1); // avoid division by zero for texts without punctuation

    let lix = (word_count as f64 / sentence_count as f64)
        + (long_word_count as f64 * 100.0 / word_count as f64);
    Some(lix)
}

/// Returns word count and estimated reading time (at 150 wpm, typical for audio/lectures).
pub(crate) fn transcript_stats(text: &str) -> (usize, usize) {
    let words = text.split_whitespace().count();
    let minutes = (words + 149) / 150; // ceiling division
    (words, minutes)
}

/// Human-readable LIX band label.
pub(crate) fn lix_label(lix: f64) -> &'static str {
    match lix as u32 {
        0..=24 => "Very easy",
        25..=34 => "Easy",
        35..=44 => "Medium",
        45..=54 => "Difficult",
        _ => "Very difficult",
    }
}

pub(crate) fn split_duration(secs: u64) -> (u64, u64) {
    (secs / 3600, (secs % 3600) / 60)
}

// ── Article extraction ────────────────────────────────────────────────────────

/// The shortest body worth treating as a real extraction. Below this, what
/// came back is boilerplate — a bot-check interstitial, a consent wall, or a
/// page whose content never rendered — not an article.
const MIN_ARTICLE_WORDS: usize = 50;

/// A readable article recovered from a page's markup.
pub(crate) struct ArticleDoc {
    pub title: String,
    pub body: String,
    /// The article's own publication date as `YYYY/MM/DD`, when the page
    /// carried a usable timestamp in its metadata.
    pub published: Option<String>,
}

/// Extracts the readable article out of `html` with a local readability pass
/// — the same scoring approach as Firefox's Reader View, run inside the
/// extension. Nothing is fetched and nothing is sent anywhere: `url` only
/// resolves relative links while the document is scored.
///
/// Returns `Err` when the markup yields no article at all, or a body too thin
/// to be one, so the caller can fall back to a source that does have the
/// content.
pub(crate) fn extract_article(html: &str, url: &str) -> Result<ArticleDoc, String> {
    let mut readability = Readability::new(html, Some(url), Some(Config::default()))
        .map_err(|e| format!("Could not parse the page's markup: {e}"))?;
    let article: Article = readability
        .parse()
        .map_err(|e| format!("No readable article found on the page: {e}"))?;

    let body = article.text_content.trim().to_string();
    let words = body.split_whitespace().count();
    if words < MIN_ARTICLE_WORDS {
        return Err(format!("The page yielded only {words} words of readable text."));
    }

    Ok(ArticleDoc {
        title: article.title.trim().to_string(),
        body,
        published: article.published_time.as_deref().and_then(normalize_date),
    })
}

/// Normalises a timestamp to the `YYYY/MM/DD` form the YourLearning form
/// expects. Handles both shapes pages actually publish: ISO-8601 and anything
/// prefixed by it (`2024-08-30`, `2024-08-30T00:00:00Z`), and RFC 2822
/// (`Fri, 07 Aug 2026 14:10:46 GMT`), which HTTP headers and a good many CMSs
/// emit. Returns `None` for anything that isn't a real date, so a junk value
/// falls through to the caller's own default rather than being filled in
/// verbatim.
fn normalize_date(raw: &str) -> Option<String> {
    let raw = raw.trim();

    if let Some(date) = raw
        .get(..10)
        .and_then(|candidate| chrono::NaiveDate::parse_from_str(candidate, "%Y-%m-%d").ok())
    {
        return Some(date.format("%Y/%m/%d").to_string());
    }

    chrono::DateTime::parse_from_rfc2822(raw)
        .ok()
        .map(|dt| dt.format("%Y/%m/%d").to_string())
}

/// Builds the analytics block for a plain "Description" body of text — the
/// shape shared by all the podcast handlers (Apple, Spotify, RSS, Vimeo).
/// Returns `None` for empty text; YouTube and articles build their own
/// (different primary label, and articles always report even without LIX).
pub(crate) fn build_description_analytics(text: &str, warning: Option<String>) -> Option<storage::AnalyticsInfo> {
    if text.trim().is_empty() {
        return None;
    }
    let (words, read_mins) = transcript_stats(text);
    let lix = compute_lix(text);
    Some(storage::AnalyticsInfo {
        primary_label: "Description".to_string(),
        primary_value: format!("{words} words  |  ~{read_mins} min read"),
        lix: lix.map(|score| storage::LixScore { score, label: lix_label(score).to_string() }),
        warning,
    })
}

// ── Helpers shared with other controllers ─────────────────────────────────────

/// Hands the prefill payload to the content script (via a well-known
/// `chrome.storage.local` key it reads on load), opens the YourLearning tab,
/// and records the entry — with its analytics — in history.
pub(crate) async fn finish_add_learning(
    url: &str,
    title: &str,
    hours: u64,
    minutes: u64,
    today: &str,
    description: &str,
    analytics: Option<storage::AnalyticsInfo>,
) -> Result<(), String> {
    let payload = serde_json::json!({
        "title": title,
        "url": url,
        "description": description,
        "today": today,
        "hours": hours,
        "minutes": minutes,
    })
    .to_string();

    storage::set_setting(PENDING_ADD_LEARNING_KEY, &payload).await?;

    browser::open_tab(YOURLEARNING_URL)
        .await
        .map_err(|e| format!("Failed to open browser: {e}"))?;

    // Record in history (best-effort — don't fail the whole flow if this errors).
    let _ = storage::add_history(url, title, hours, minutes, today, analytics).await;

    Ok(())
}

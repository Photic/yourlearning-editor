use crate::browser::sleep;
use crate::{browser, http, storage};
use dioxus_logger::tracing;

pub(crate) const YOURLEARNING_URL: &str = "https://yourlearning.ibm.com/add-learning";
const PENDING_ADD_LEARNING_KEY: &str = "PENDING_ADD_LEARNING";

// ── HF Inference API (bart-large-cnn) ────────────────────────────────────────

/// Number of retries after a network/timeout failure, before giving up.
const HF_MAX_NETWORK_RETRIES: u32 = 2;
/// Base delay for the exponential backoff between retries: 1s, 2s, ...
const HF_RETRY_BASE_DELAY_MS: i32 = 1_000;

/// POSTs to `url`, retrying on network/timeout failures with exponential
/// backoff up to `HF_MAX_NETWORK_RETRIES` times. This only covers transport
/// failures (e.g. the request never got a response) — HF's own "model
/// loading" error payload comes back as `Ok` and is handled by the caller.
async fn post_json_with_retry(
    url: &str,
    headers: &[(&str, &str)],
    body: &serde_json::Value,
    timeout_ms: u32,
) -> Result<String, String> {
    let mut attempt = 0;
    loop {
        match http::post_json(url, headers, body, timeout_ms).await {
            Ok(raw) => return Ok(raw),
            Err(e) if attempt < HF_MAX_NETWORK_RETRIES => {
                let delay_ms = HF_RETRY_BASE_DELAY_MS * 2i32.pow(attempt);
                tracing::debug!(
                    "[HF] Request failed ({e}) — retrying in {delay_ms}ms (attempt {}/{})",
                    attempt + 1,
                    HF_MAX_NETWORK_RETRIES
                );
                sleep(delay_ms).await;
                attempt += 1;
            }
            Err(e) => {
                tracing::debug!("[HF] Request failed after {} attempts: {e}", attempt + 1);
                return Err("Connection to HuggingFace failed. Please try again.".to_string());
            }
        }
    }
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
    let raw = post_json_with_retry(
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

        let raw2 = post_json_with_retry(
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

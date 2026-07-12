use chrono::{Local, NaiveDate};

use super::youtube_learning::{
    compute_lix, finish_add_learning, hf_api_token, lix_label, summarize_with_bart, transcript_stats,
};

// ── HTTP helpers ──────────────────────────────────────────────────────────────

/// Fetches `url` and returns the raw HTML body.
async fn fetch_html(url: &str) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (compatible; yourlearning-editor)")
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {e}"))?;

    client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("Failed to fetch page: {e}"))?
        .text()
        .await
        .map_err(|e| format!("Failed to read response body: {e}"))
}

/// Fetches the plain-text reader view of `url` via Jina Reader
/// (`https://r.jina.ai/<url>`).  Returns `None` on any error.
///
/// Jina Reader renders JS-heavy pages server-side and returns clean
/// markdown-like text, making it ideal as a fallback for SPAs.
/// The response starts with `Title: …\nURL Source: …\n\nMarkdown Content:\n`
/// followed by the article body.
async fn fetch_via_jina(url: &str) -> Option<(String, String)> {
    let jina_url = format!("https://r.jina.ai/{url}");
    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (compatible; yourlearning-editor)")
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .ok()?;

    let text = client
        .get(&jina_url)
        .header("Accept", "text/plain")
        .send()
        .await
        .ok()?
        .text()
        .await
        .ok()?;

    // Parse the Jina response header lines.
    // Format:
    //   Title: <title>
    //   URL Source: <url>
    //   <blank line>
    //   Markdown Content:
    //   <body text>
    let mut title = String::new();
    let mut body_start = 0usize;
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(t) = trimmed.strip_prefix("Title:") {
            title = t.trim().to_string();
        }
        if trimmed == "Markdown Content:" {
            body_start = text.find("Markdown Content:")
                .map(|i| i + "Markdown Content:".len())
                .unwrap_or(0);
            break;
        }
    }

    let body = if body_start > 0 {
        text[body_start..].trim().to_string()
    } else {
        text.clone()
    };

    if body.split_whitespace().count() < 50 {
        return None;
    }

    Some((title, body))
}

/// Extracts the text content of the first `<h1>` tag found in `html`.
/// Falls back to the `<title>` tag, then to an empty string.
fn extract_title(html: &str) -> String {
    if let Some(title) = extract_tag_text(html, "h1") {
        if !title.is_empty() {
            return title;
        }
    }
    extract_tag_text(html, "title").unwrap_or_default()
}

/// Finds the first `<tag>…</tag>` block (case-insensitive open tag) and
/// returns its inner text with all child tags stripped.
fn extract_tag_text(html: &str, tag: &str) -> Option<String> {
    let lower = html.to_lowercase();
    let open = format!("<{tag}");
    let close = format!("</{tag}>");

    let tag_start = lower.find(&open)?;
    // Skip past the closing `>` of the opening tag.
    let content_start = html[tag_start..].find('>')? + tag_start + 1;
    let content_end = lower[content_start..].find(&close)? + content_start;

    Some(strip_tags(&html[content_start..content_end]).trim().to_string())
}

/// Strips all HTML tags from a string, including the full contents of any
/// `<script>` and `<style>` blocks (not just their tags).
/// Decodes HTML entities and preserves word boundaries.
fn strip_tags(html: &str) -> String {
    let lower = html.to_lowercase();
    let mut out = String::with_capacity(html.len());
    let mut pos = 0usize;

    'outer: while pos < lower.len() {
        // Skip entire <script …> … </script> and <style …> … </style> blocks.
        for (open, close) in &[("<script", "</script>"), ("<style", "</style>")] {
            if lower[pos..].starts_with(open) {
                pos += match lower[pos..].find(close) {
                    Some(end) => end + close.len(),
                    None => lower.len() - pos, // malformed — skip rest
                };
                continue 'outer;
            }
        }

        if lower.as_bytes()[pos] == b'<' {
            // Skip to end of tag, emit a space for word boundary.
            match lower[pos..].find('>') {
                Some(end) => { pos += end + 1; out.push(' '); }
                None => break,
            }
        } else {
            let c = html[pos..].chars().next().unwrap();
            out.push(c);
            pos += c.len_utf8();
        }
    }

    decode_entities(&out)
}

fn decode_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ")
}

/// Extracts readable body text from an HTML document.
///
/// Strategy: pull all `<p>` tag content and join it.  If that yields fewer
/// than 100 words we fall back to stripping the entire `<body>`.  This covers
/// most articles and blog posts without needing a full HTML parser.
fn extract_body_text(html: &str) -> String {
    let lower = html.to_lowercase();
    let mut paragraphs = String::new();
    let mut pos = 0usize;

    while pos < lower.len() {
        // Find the next <p or <p> opening tag.
        let p_start = match lower[pos..].find("<p") {
            Some(i) => pos + i,
            None => break,
        };
        // Ensure it's actually a <p>, <p …>, or <p\n> tag and not e.g. <pre>, <path>.
        // Valid continuations after "<p": whitespace, ">", or "/" (self-closing).
        // Anything else (a letter like 'r' in <pre> or 'a' in <path>) means a different tag.
        let after_p = p_start + 2;
        if after_p < lower.len() {
            let next_ch = lower.as_bytes()[after_p];
            if next_ch.is_ascii_alphabetic() {
                pos = after_p;
                continue;
            }
        }
        // Skip past the closing `>` of the opening tag.
        let tag_end = match lower[p_start..].find('>') {
            Some(i) => p_start + i + 1,
            None => break,
        };
        // Find the matching </p>.
        let close_pos = match lower[tag_end..].find("</p>") {
            Some(i) => tag_end + i,
            None => {
                pos = tag_end;
                continue;
            }
        };
        let inner = strip_tags(&html[tag_end..close_pos]);
        let inner = inner.trim().to_string();
        if !inner.is_empty() {
            if !paragraphs.is_empty() {
                paragraphs.push(' ');
            }
            paragraphs.push_str(&inner);
        }
        pos = close_pos + 4; // advance past </p>
    }

    // Fallback: try semantic containers before stripping the whole body.
    if paragraphs.split_whitespace().count() < 100 {
        // Ordered preference: <article>, <main>, <body>
        for container in &["<article", "<main", "<body"] {
            if let Some(start) = lower.find(container) {
                let text = strip_tags(&html[start..]);
                if text.split_whitespace().count() >= 50 {
                    return text;
                }
            }
        }
        // Last resort: strip everything.
        return strip_tags(html);
    }

    paragraphs
}

// ── Public handler ────────────────────────────────────────────────────────────

/// Handles any non-YouTube URL: fetch the page, extract title + body text,
/// optionally summarise with BART, compute LIX, then fire the relay.
pub(crate) async fn run_article(
    app: &tauri::AppHandle,
    url: &str,
    date_override: &str,
    use_ai_summary: bool,
) -> Result<String, String> {
    println!("[Article] Fetching {url}");
    let html = fetch_html(url).await?;

    let html_title = extract_title(&html);
    let html_body = extract_body_text(&html);

    // If the direct fetch yielded too little text (JS-rendered SPA), fall back
    // to Jina Reader which server-side renders the page and returns clean text.
    let (title, body_text) = if html_body.split_whitespace().count() < 200 {
        println!("[Article] Thin content ({} words) — trying Jina Reader…", html_body.split_whitespace().count());
        match fetch_via_jina(url).await {
            Some((jina_title, jina_body)) => {
                println!("[Article] Jina Reader: {} words", jina_body.split_whitespace().count());
                let title = if jina_title.is_empty() { html_title } else { jina_title };
                (title, jina_body)
            }
            None => {
                println!("[Article] Jina Reader failed — using direct fetch content");
                (html_title, html_body)
            }
        }
    } else {
        (html_title, html_body)
    };

    println!("[Article] Title: {title:?}");
    println!("[Article] Body text ({} chars)", body_text.len());

    // ── Compute LIX / reading time (always, no token needed) ─────────────────
    let lix = compute_lix(&body_text);
    let (words, read_mins) = transcript_stats(&body_text);

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
    let description = if use_ai_summary && hf_api_token(app).is_some() {
        let summary = summarize_with_bart(app, &body_text).await;
        println!("[Article] Summary: {summary:?}");
        summary.unwrap_or_default()
    } else {
        String::new()
    };

    // ── Analytics line ───────────────────────────────────────────────────────
    // Use the same adjusted reading time that was sent to YourLearning.
    let display_mins = total_read_mins;
    let transcript_info = Some(match lix {
        Some(score) => format!(
            "  Article:     {} words  |  ~{} min read (@ {}wpm)\n  LIX score:   {:.1} — {}",
            words, display_mins, wpm, score, lix_label(score)
        ),
        None => format!("  Article:     {} words  |  ~{} min read", words, display_mins),
    });

    // ── Date ─────────────────────────────────────────────────────────────────
    let today = if !date_override.trim().is_empty() {
        NaiveDate::parse_from_str(date_override.trim(), "%Y-%m-%d")
            .map(|d| d.format("%Y/%m/%d").to_string())
            .unwrap_or_else(|_| date_override.trim().to_string())
    } else {
        Local::now().format("%Y/%m/%d").to_string()
    };

    finish_add_learning(app, url, &title, hours, minutes, &today, &description, transcript_info)
        .await
}

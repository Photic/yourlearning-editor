use chrono::Local;
use std::process::Command;

const YOURLEARNING_URL: &str = "https://yourlearning.ibm.com/add-learning";

/// JS template injected into Chrome to auto-fill the YourLearning form.
/// Placeholders (ALL_CAPS) are replaced with JSON-encoded values before injection.
const JS_TEMPLATE: &str = r#"(function poll() {
  var ready = document.readyState === 'complete'
           && document.querySelectorAll('input[type="text"]').length >= 2
           && document.querySelector('textarea') !== null;
  if (!ready) { setTimeout(poll, 200); return; }

  function setVal(el, value, withBlur) {
    if (!el) return;
    var proto = el.tagName === 'TEXTAREA'
      ? window.HTMLTextAreaElement.prototype
      : window.HTMLInputElement.prototype;
    var setter = Object.getOwnPropertyDescriptor(proto, 'value').set;
    el.dispatchEvent(new Event('focus', { bubbles: true }));
    setter.call(el, value);
    el.dispatchEvent(new Event('input',  { bubbles: true }));
    el.dispatchEvent(new Event('change', { bubbles: true }));
    if (withBlur) el.dispatchEvent(new Event('blur', { bubbles: true }));
  }

  var textInputs = document.querySelectorAll('input[type="text"]');
  setVal(textInputs[0], TITLE);
  if (textInputs.length >= 2) setVal(textInputs[1], URL);

  setVal(document.querySelector('textarea'), DESC);

  var dateInputs = document.querySelectorAll('input[placeholder="yyyy/mm/dd"]');
  if (dateInputs[0]) setVal(dateInputs[0], TODAY, true);
  if (dateInputs[1]) setVal(dateInputs[1], TODAY, true);

  var numInputs = document.querySelectorAll('input[type="number"]');
  if (numInputs[0]) setVal(numInputs[0], HOURS);
  if (numInputs[1]) setVal(numInputs[1], MINUTES);

  var activityDropdown = document.querySelector('select, [role="combobox"], [role="listbox"], button[aria-haspopup="listbox"]');
  if (activityDropdown) {
    activityDropdown.click();
    setTimeout(function() {
      var options = document.querySelectorAll('[role="option"], option');
      for (var i = 0; i < options.length; i++) {
        if (options[i].textContent.trim().toLowerCase() === 'video') {
          options[i].click();
          break;
        }
      }
    }, 300);
  }
})();"#;

// ── YouTube metadata fetch ────────────────────────────────────────────────────

struct VideoMeta {
    title: String,
    author: String,
    duration_secs: u64,
    description: String,
}

/// Fetches the YouTube watch page and extracts `ytInitialPlayerResponse` from
/// the embedded inline JSON, then returns the fields we need.
async fn fetch_video_meta(url: &str) -> Result<VideoMeta, String> {
    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0 Safari/537.36")
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {e}"))?;

    let html = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("Failed to fetch YouTube page: {e}"))?
        .text()
        .await
        .map_err(|e| format!("Failed to read response body: {e}"))?;

    let json_obj = extract_player_response(&html)
        .ok_or("Could not find ytInitialPlayerResponse in the page. The URL may be invalid.")?;

    let details = &json_obj["videoDetails"];

    let title = details["title"].as_str().unwrap_or("").to_string();
    let author = details["author"].as_str().unwrap_or("").to_string();
    let duration_secs = details["lengthSeconds"]
        .as_str()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);
    let description = details["shortDescription"]
        .as_str()
        .unwrap_or("")
        .to_string();

    Ok(VideoMeta { title, author, duration_secs, description })
}

/// Locates `ytInitialPlayerResponse = {...}` in the page HTML and parses the
/// JSON object using a brace counter (avoids pulling in a full HTML parser).
fn extract_player_response(html: &str) -> Option<serde_json::Value> {
    let marker = "ytInitialPlayerResponse = ";
    let start = html.find(marker)? + marker.len();
    let slice = &html[start..];

    // Walk forward counting braces to find the matching closing `}`
    let mut depth = 0usize;
    let mut end = 0usize;
    for (i, ch) in slice.char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end = i + 1;
                    break;
                }
            }
            _ => {}
        }
    }

    serde_json::from_str(&slice[..end]).ok()
}

// ── Field processing ──────────────────────────────────────────────────────────

/// `"channel: title"` or just `"title"` when author is absent.
fn format_title(author: &str, title: &str) -> String {
    if author.is_empty() {
        title.to_string()
    } else {
        format!("{author}: {title}")
    }
}

/// Duration in whole hours and remaining minutes.
fn split_duration(secs: u64) -> (u64, u64) {
    (secs / 3600, (secs % 3600) / 60)
}

/// Returns the cleaned description, or an empty string when the description is
/// dominated by links/short labels (≥70 % of non-empty lines).
fn clean_description(raw: &str) -> String {
    let raw = raw.trim();
    let lines: Vec<&str> = raw.lines().collect();
    let non_empty: Vec<&str> = lines.iter().copied().filter(|l| !l.trim().is_empty()).collect();
    let total = non_empty.len().max(1);

    let link_related = non_empty.iter().filter(|l| is_link_related(l)).count();
    if link_related as f64 / total as f64 >= 0.7 {
        return String::new();
    }

    lines
        .iter()
        .filter(|l| !l.contains("http://") && !l.contains("https://"))
        .cloned()
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .chars()
        .take(1000)
        .collect()
}

/// A line is "link-related" if it contains a URL or is a short label (≤60 chars).
fn is_link_related(line: &str) -> bool {
    line.contains("http://") || line.contains("https://") || line.trim().len() <= 60
}

// ── JS building ───────────────────────────────────────────────────────────────

fn build_js(title: &str, url: &str, description: &str, today: &str, hours: u64, minutes: u64) -> String {
    JS_TEMPLATE
        .replace("TITLE",   &serde_json::to_string(title).unwrap())
        .replace("URL",     &serde_json::to_string(url).unwrap())
        .replace("DESC",    &serde_json::to_string(description).unwrap())
        .replace("TODAY",   &serde_json::to_string(today).unwrap())
        .replace("HOURS",   &serde_json::to_string(&hours.to_string()).unwrap())
        .replace("MINUTES", &serde_json::to_string(&minutes.to_string()).unwrap())
}

// ── Public command ────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn run_add_learning(url: String) -> Result<String, String> {
    // Strip extra query params after the video ID (e.g. &t=235s).
    let url = url.trim().splitn(2, '&').next().unwrap_or(&url).to_string();

    // ── Fetch metadata ───────────────────────────────────────────────────────
    let meta = fetch_video_meta(&url).await?;
    let title = format_title(&meta.author, &meta.title);
    let (hours, minutes) = split_duration(meta.duration_secs);
    let description = clean_description(&meta.description);
    let today = Local::now().format("%Y/%m/%d").to_string();

    let summary = format!(
        "\n{sep}\n  YourLearning — Add Personal Learning\n{sep}\n  Title:    {title}\n  URL:      {url}\n  Duration: {hours}h {minutes}m\n  Date:     {today}\n{sep}\n",
        sep = "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    );

    // ── Open Chrome ──────────────────────────────────────────────────────────
    Command::new("open")
        .args(["-a", "Google Chrome", YOURLEARNING_URL])
        .spawn()
        .map_err(|e| format!("Failed to open Chrome: {e}"))?;

    // Brief pause so Chrome finishes opening the tab before JS injection.
    std::thread::sleep(std::time::Duration::from_secs(2));

    // ── Inject JS via osascript ───────────────────────────────────────────────
    let js = build_js(&title, &url, &description, &today, hours, minutes);
    let applescript = format!(
        "tell application \"Google Chrome\"\nactivate\nexecute active tab of front window javascript \"{}\"\nend tell",
        js.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n")
    );

    let osa_status = Command::new("osascript")
        .args(["-e", &applescript])
        .status()
        .map_err(|e| format!("Failed to run osascript: {e}"))?;

    if !osa_status.success() {
        return Err("osascript failed to inject JS into Chrome.".to_string());
    }

    Ok(format!("{summary}✓ Form will fill automatically once the page finishes loading."))
}

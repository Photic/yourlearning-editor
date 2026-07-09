use chrono::Local;
use serde_json::Value;
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

// ── Metadata extraction ───────────────────────────────────────────────────────

/// `"channel: title"` or just `"title"` when channel is absent.
fn parse_title(meta: &Value) -> String {
    let channel = meta["uploader"]
        .as_str()
        .or_else(|| meta["channel"].as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let title = meta["title"].as_str().unwrap_or("").trim().to_string();
    if channel.is_empty() {
        title
    } else {
        format!("{channel}: {title}")
    }
}

/// Duration in whole hours and remaining minutes.
fn parse_duration(meta: &Value) -> (u64, u64) {
    let secs = meta["duration"].as_f64().unwrap_or(0.0) as u64;
    (secs / 3600, (secs % 3600) / 60)
}

/// Returns the cleaned description, or an empty string when the description is
/// dominated by links/short labels (≥70 % of non-empty lines).
fn parse_description(meta: &Value) -> String {
    let raw = meta["description"].as_str().unwrap_or("").trim().to_string();

    let lines: Vec<&str> = raw.lines().collect();
    let non_empty: Vec<&str> = lines.iter().copied().filter(|l| !l.trim().is_empty()).collect();
    let total = non_empty.len().max(1);

    let link_related = non_empty.iter().filter(|l| is_link_related(l)).count();

    if link_related as f64 / total as f64 >= 0.7 {
        return String::new();
    }

    // Strip lines that contain URLs, then truncate to 1000 chars.
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

    // ── Check yt-dlp ─────────────────────────────────────────────────────────
    if Command::new("which").arg("yt-dlp").output().map(|o| !o.status.success()).unwrap_or(true) {
        return Err("yt-dlp is not installed. Run: brew install yt-dlp".to_string());
    }

    // ── Fetch metadata ───────────────────────────────────────────────────────
    let yt_output = Command::new("yt-dlp")
        .args(["--dump-json", "--no-download", "--quiet", &url])
        .output()
        .map_err(|e| format!("Failed to run yt-dlp: {e}"))?;

    if !yt_output.status.success() || yt_output.stdout.is_empty() {
        return Err("Could not fetch metadata. Check the URL and try again.".to_string());
    }

    let meta: Value = serde_json::from_slice(&yt_output.stdout)
        .map_err(|e| format!("Failed to parse yt-dlp output: {e}"))?;

    // ── Parse fields ─────────────────────────────────────────────────────────
    let title = parse_title(&meta);
    let (hours, minutes) = parse_duration(&meta);
    let description = parse_description(&meta);
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

#!/usr/bin/env bash
# add-learning.sh
# Usage: ./add-learning.sh <youtube-url>
#
# Fetches metadata from a YouTube video using yt-dlp, then auto-fills the
# YourLearning "Add personal learning" form in Chrome.

set -euo pipefail

YOURLEARNING_URL="https://yourlearning.ibm.com/add-learning"

# ── Validate input ────────────────────────────────────────────────────────────
if [[ $# -lt 1 ]]; then
  echo "Usage: $0 <youtube-url>"
  exit 1
fi

# Strip any query parameters after the video ID (e.g. &t=235s)
URL=$(echo "$1" | sed 's/&.*//')

# ── Check dependencies ────────────────────────────────────────────────────────
if ! command -v yt-dlp &>/dev/null; then
  echo "Error: yt-dlp is not installed. Run: brew install yt-dlp"
  exit 1
fi

# ── Fetch metadata ────────────────────────────────────────────────────────────
echo "Fetching metadata for: $URL"
JSON=$(yt-dlp --dump-json --no-download --quiet "$URL" 2>/dev/null)

if [[ -z "$JSON" ]]; then
  echo "Error: Could not fetch metadata. Check the URL and try again."
  exit 1
fi

# ── Parse fields ──────────────────────────────────────────────────────────────
TITLE=$(echo "$JSON" | python3 -c "
import sys, json
d = json.load(sys.stdin)
channel = d.get('uploader', d.get('channel', ''))
title   = d.get('title', '')
print(f'{channel}: {title}' if channel else title)
")
DURATION_SEC=$(echo "$JSON" | python3 -c "import sys,json; d=json.load(sys.stdin); print(int(d.get('duration',0)))")

DESCRIPTION=$(echo "$JSON" | python3 -c "
import sys, json, re
d = json.load(sys.stdin)
desc = d.get('description', '').strip()

# Remove blank lines for analysis
lines = desc.splitlines()
non_empty = [l for l in lines if l.strip()]

# A line is 'link-related' if it contains a URL or is a short label (<=60 chars)
def is_link_related(l):
    return bool(re.search(r'https?://', l)) or len(l.strip()) <= 60

link_related = sum(1 for l in non_empty if is_link_related(l))
total = len(non_empty) if non_empty else 1

# Blank out if 70%+ of non-empty lines are link-related
if total == 0 or link_related / total >= 0.7:
    desc = ''
else:
    lines = [l for l in lines if not re.search(r'https?://', l)]
    desc = '\n'.join(lines).strip()[:1000]

print(desc)
")

HOURS=$(( DURATION_SEC / 3600 ))
MINUTES=$(( (DURATION_SEC % 3600) / 60 ))
TODAY=$(date +%Y/%m/%d)

# ── Print summary ─────────────────────────────────────────────────────────────
echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "  YourLearning — Add Personal Learning"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "  Title:     $TITLE"
echo "  URL:       $URL"
echo "  Duration:  ${HOURS}h ${MINUTES}m"
echo "  Date:      $TODAY"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""

# ── Build JS via Python (handles all escaping safely) ─────────────────────────
JS_FILE=$(mktemp /tmp/yl-fill-XXXXXX.js)
trap 'rm -f "$JS_FILE"' EXIT

python3 - "$TITLE" "$URL" "$DESCRIPTION" "$TODAY" "$HOURS" "$MINUTES" <<'PYEOF' > "$JS_FILE"
import sys, json

title       = sys.argv[1]
url         = sys.argv[2]
description = sys.argv[3]
today       = sys.argv[4]
hours       = sys.argv[5]
minutes     = sys.argv[6]

# Poll until the React form has finished rendering, then fill all fields.
js = """
(function poll() {
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
})();
""" \
    .replace("TITLE",   json.dumps(title))       \
    .replace("URL",     json.dumps(url))          \
    .replace("DESC",    json.dumps(description))  \
    .replace("TODAY",   json.dumps(today))        \
    .replace("HOURS",   json.dumps(hours))        \
    .replace("MINUTES", json.dumps(minutes))

print(js)
PYEOF

# ── Open Chrome and inject the polling JS once the tab has navigated ──────────
echo "Opening YourLearning form in Chrome..."
open -a "Google Chrome" "$YOURLEARNING_URL"

# Brief pause so Chrome finishes opening the tab before we inject.
# The JS itself polls for the React form — no further waiting needed.
sleep 2

JS_CONTENT=$(cat "$JS_FILE")

osascript - "$JS_CONTENT" <<'ASEOF'
on run argv
  set jsCode to item 1 of argv
  tell application "Google Chrome"
    activate
    execute active tab of front window javascript jsCode
  end tell
end run
ASEOF

echo "✓ Form will fill automatically once the page finishes loading."

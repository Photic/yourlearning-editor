// YourLearning Editor — content script
// Runs on https://yourlearning.ibm.com/add-learning
//
// Flow:
//   1. Fetch prefill data from the relay server the desktop app started
//      on localhost:19421/data (retries for up to 5 seconds)
//   2. Wait for the React form to finish rendering
//   3. Fill every field using React's synthetic event system

(async function () {
  // ── 1. Fetch data from relay (with retries) ───────────────────────────────
  let data;
  for (let attempt = 0; attempt < 10; attempt++) {
    try {
      const resp = await fetch("http://localhost:19421/data");
      if (resp.ok) {
        data = await resp.json();
        break;
      }
    } catch {
      // Relay not ready yet — wait and retry
    }
    await sleep(500);
  }

  if (!data) {
    // App wasn't running when the page loaded — nothing to fill
    console.debug("[YourLearning Editor] No relay data found, skipping autofill.");
    return;
  }

  console.debug("[YourLearning Editor] Got relay data:", data);

  // ── 2. Wait for the React form ────────────────────────────────────────────
  try {
    await waitFor(() =>
      document.querySelectorAll('input[type="text"]').length >= 2 &&
      document.querySelector('textarea') !== null
    );
  } catch (e) {
    console.error("[YourLearning Editor] Timed out waiting for form:", e);
    return;
  }

  console.debug("[YourLearning Editor] Form found, filling fields…");

  // ── 3. Fill fields ────────────────────────────────────────────────────────
  const textInputs = document.querySelectorAll('input[type="text"]');
  setVal(textInputs[0], data.title);
  if (textInputs.length >= 2) setVal(textInputs[1], data.url);

  setVal(document.querySelector('textarea'), data.description);

  const dateInputs = document.querySelectorAll('input[placeholder="yyyy/mm/dd"]');
  if (dateInputs[0]) setVal(dateInputs[0], data.today, true);
  if (dateInputs[1]) setVal(dateInputs[1], data.today, true);

  const numInputs = document.querySelectorAll('input[type="number"]');
  if (numInputs[0]) setVal(numInputs[0], String(data.hours));
  if (numInputs[1]) setVal(numInputs[1], String(data.minutes));

  const dropdown = document.querySelector(
    'select, [role="combobox"], [role="listbox"], button[aria-haspopup="listbox"]'
  );
  if (dropdown) {
    dropdown.click();
    await sleep(300);
    const options = document.querySelectorAll('[role="option"], option');
    for (const opt of options) {
      if (opt.textContent.trim().toLowerCase() === "video") {
        opt.click();
        break;
      }
    }
  }

  console.debug("[YourLearning Editor] Form filled successfully.");

  // ── Helpers ───────────────────────────────────────────────────────────────

  function setVal(el, value, withBlur = false) {
    if (!el) return;
    const proto =
      el.tagName === "TEXTAREA"
        ? window.HTMLTextAreaElement.prototype
        : window.HTMLInputElement.prototype;
    const setter = Object.getOwnPropertyDescriptor(proto, "value").set;
    el.dispatchEvent(new Event("focus", { bubbles: true }));
    setter.call(el, value);
    el.dispatchEvent(new Event("input", { bubbles: true }));
    el.dispatchEvent(new Event("change", { bubbles: true }));
    if (withBlur) el.dispatchEvent(new Event("blur", { bubbles: true }));
  }

  function sleep(ms) {
    return new Promise((r) => setTimeout(r, ms));
  }

  function waitFor(predicate, interval = 200, timeout = 15000) {
    return new Promise((resolve, reject) => {
      const start = Date.now();
      const id = setInterval(() => {
        if (predicate()) {
          clearInterval(id);
          resolve();
        } else if (Date.now() - start > timeout) {
          clearInterval(id);
          reject(new Error("Timed out waiting for form"));
        }
      }, interval);
    });
  }
})();

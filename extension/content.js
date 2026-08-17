// YourLearning Editor — content script
// Runs on https://yourlearning.ibm.com/add-learning
//
// Flow:
//   1. Read the prefill payload the popup left in chrome.storage.local under
//      PENDING_ADD_LEARNING (written before the tab was opened, so it's
//      already there by the time this script runs at document_idle), then
//      clear it so a stale payload can't autofill a later visit.
//   2. Wait for the React form to finish rendering
//   3. Fill every field using React's synthetic event system

(async function () {
  // ── 1. Read the pending payload ────────────────────────────────────────────
  const stored = await chrome.storage.local.get("PENDING_ADD_LEARNING");
  const raw = stored.PENDING_ADD_LEARNING;

  if (!raw) {
    // The popup wasn't used before this page loaded — nothing to fill.
    console.debug("[YourLearning Editor] No pending payload, skipping autofill.");
    return;
  }

  await chrome.storage.local.remove("PENDING_ADD_LEARNING");

  let data;
  try {
    data = JSON.parse(raw);
  } catch (e) {
    console.error("[YourLearning Editor] Could not parse pending payload:", e);
    return;
  }

  console.debug("[YourLearning Editor] Got pending payload:", data);

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

  await selectActivityType("video");

  console.debug("[YourLearning Editor] Form filled successfully.");

  // Opens the Activity Type dropdown and clicks the option matching `label`.
  async function selectActivityType(label) {
    // Find the dropdown trigger — look for a button/div that contains the
    // placeholder text or has a listbox/combobox role.
    const trigger =
      document.querySelector('[aria-haspopup="listbox"]') ||
      document.querySelector('[role="combobox"]') ||
      [...document.querySelectorAll("button")].find((b) =>
        b.textContent.trim().toLowerCase().includes("activity")
      );

    if (!trigger) {
      console.debug("[YourLearning Editor] Activity type trigger not found.");
      return;
    }

    trigger.click();
    console.debug("[YourLearning Editor] Dropdown triggered.");

    // Wait for options to appear in the DOM.
    let options = [];
    for (let i = 0; i < 10; i++) {
      await sleep(200);
      options = [
        ...document.querySelectorAll('[role="option"]'),
        ...document.querySelectorAll('[role="menuitem"]'),
        ...document.querySelectorAll("li"),
      ];
      if (options.length > 0) break;
    }

    console.debug("[YourLearning Editor] Options found:", options.map(o => o.textContent.trim()));

    const match = options.find(
      (o) => o.textContent.trim().toLowerCase() === label.toLowerCase()
    );
    if (match) {
      match.click();
      console.debug("[YourLearning Editor] Selected:", match.textContent.trim());
    } else {
      console.debug("[YourLearning Editor] Option not found:", label);
    }
  }

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

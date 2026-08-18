// YourLearning Editor — background service worker (Manifest V3)
//
// Runs the add-learning pipeline (fetch metadata, summarize, hand off to the
// content script) here rather than in the popup, since Chrome tears the
// popup's JS down the instant it loses focus — the service worker survives
// as long as there's pending work (a pending fetch, a pending message
// response), so a task keeps running even if the popup gets closed mid-flight.

import init, {
  run_add_learning,
  run_focus_page_learning,
  focus_page_needs_consent,
} from "./background_wasm.js";

const ready = init();

chrome.runtime.onMessage.addListener((message, _sender, sendResponse) => {
  if (message?.type === "add_learning") {
    (async () => {
      await ready;
      try {
        await run_add_learning(message.url, message.dateOverride, message.useAiSummary);
        sendResponse({ ok: true });
      } catch (error) {
        sendResponse({ ok: false, message: String(error) });
      }
    })();

    return true; // keep the message channel open for the async sendResponse above
  }

  if (message?.type === "add_focus_page_learning") {
    (async () => {
      await ready;
      try {
        await run_focus_page_learning(message.dateOverride, message.useAiSummary);
        sendResponse({ ok: true });
      } catch (error) {
        sendResponse({ ok: false, message: String(error) });
      }
    })();

    return true; // keep the message channel open for the async sendResponse above
  }

  // Asked by the in-page panel before it captures anything: the popup shows a
  // consent toast under the same conditions, but it decides them with Rust it
  // can call directly. A content script can't, so it asks here instead of
  // reimplementing the known-site list in JS.
  if (message?.type === "focus_page_consent_check") {
    (async () => {
      await ready;
      try {
        const needsConsent = await focus_page_needs_consent(message.useAiSummary);
        sendResponse({ ok: true, needsConsent });
      } catch (error) {
        sendResponse({ ok: false, message: String(error) });
      }
    })();

    return true; // keep the message channel open for the async sendResponse above
  }

  return false;
});

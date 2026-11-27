import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { render } from "svelte/server";
import ToastStack from "../src/parts/ToastStack.svelte";
import { makeToast, pauseToast, pushToast, resumeToast } from "../src/lib/toasts.js";

test("makeToast fills defaults and gives timed toasts five seconds", () => {
  const toast = makeToast({ title: "Saved" }, 7);
  assert.equal(toast.id, 7);
  assert.equal(toast.tone, "info");
  assert.equal(toast.title, "Saved");
  assert.equal(toast.body, "");
  assert.deepEqual(toast.actions, []);
  assert.equal(toast.sticky, false);
  assert.equal(toast.remainingMs, 5000);
  assert.equal(makeToast({ durationMs: 1200 }, 1).remainingMs, 1200);
  assert.equal(makeToast(undefined, 2).title, "Notice");
});

test("error toasts, toasts with actions and sticky toasts stay until dismissed", () => {
  assert.equal(makeToast({ tone: "error", title: "Save failed" }, 1).sticky, true);
  assert.equal(makeToast({ actions: [{ label: "Undo" }] }, 2).sticky, true);
  assert.equal(makeToast({ sticky: true }, 3).sticky, true);
  assert.equal(makeToast({ tone: "warning" }, 4).sticky, false);
  assert.equal(makeToast({ tone: "error" }, 5).remainingMs, null);
});

test("pushToast puts the newest toast first", () => {
  const first = makeToast({ title: "one" }, 1);
  const second = makeToast({ title: "two" }, 2);
  assert.deepEqual(pushToast(pushToast([], first), second).map(t => t.id), [2, 1]);
});

test("four errors in a row all stay on screen", () => {
  let list = [];
  for (let id = 1; id <= 4; id += 1) list = pushToast(list, makeToast({ tone: "error" }, id));
  assert.deepEqual(list.map(t => t.id), [4, 3, 2, 1]);
});

test("a fourth info toast drops the oldest info toast, never an error", () => {
  let list = [];
  list = pushToast(list, makeToast({ title: "info 1" }, 1));
  list = pushToast(list, makeToast({ tone: "error" }, 2));
  list = pushToast(list, makeToast({ title: "info 3" }, 3));
  list = pushToast(list, makeToast({ title: "info 4" }, 4));
  assert.deepEqual(list.map(t => t.id), [4, 3, 2]);
  list = pushToast(list, makeToast({ title: "info 5" }, 5));
  assert.deepEqual(list.map(t => t.id), [5, 4, 2]);
});

test("pushToast honours a custom cap", () => {
  let list = [];
  for (let id = 1; id <= 3; id += 1) list = pushToast(list, makeToast({}, id), 2);
  assert.deepEqual(list.map(t => t.id), [3, 2]);
});

test("paused time is not counted toward the remaining duration", () => {
  let toast = makeToast({ durationMs: 5000 }, 1);
  toast = resumeToast(toast, 1000);
  toast = pauseToast(toast, 3000);
  assert.equal(toast.remainingMs, 3000);
  // Ten seconds of hovering later the toast still has its three seconds left.
  toast = resumeToast(toast, 13000);
  assert.equal(toast.remainingMs, 3000);
  toast = pauseToast(toast, 14000);
  assert.equal(toast.remainingMs, 2000);
});

test("pausing twice does not count time twice and never goes below zero", () => {
  let toast = resumeToast(makeToast({ durationMs: 1000 }, 1), 0);
  toast = pauseToast(toast, 400);
  assert.equal(pauseToast(toast, 900).remainingMs, 600);
  assert.equal(pauseToast(resumeToast(toast, 0), 5000).remainingMs, 0);
});

test("pause and resume leave sticky toasts alone", () => {
  const sticky = makeToast({ tone: "error" }, 1);
  assert.equal(resumeToast(sticky, 10), sticky);
  assert.equal(pauseToast(sticky, 20), sticky);
});

test("the notification region is mounted before any toast arrives", () => {
  const html = render(ToastStack).body;
  assert.match(html, /<div[^>]*role="region"[^>]*aria-label="Notifications"/);
  assert.match(html, /<div[^>]*role="status"[^>]*aria-live="polite"/);
  assert.match(html, /<div[^>]*role="alert"/);
});

test("error toasts live beside the polite region, never inside it", () => {
  const html = render(ToastStack).body;
  const polite = html.indexOf('aria-live="polite"');
  const alert = html.indexOf('role="alert"');
  assert.ok(polite >= 0 && alert >= 0);
  // Walk the markup between the two openings: every element the polite one opened is closed again.
  const start = Math.min(polite, alert);
  const between = html.slice(html.lastIndexOf("<div", start), html.lastIndexOf("<div", Math.max(polite, alert)));
  const opened = (between.match(/<div\b/g) ?? []).length;
  const closed = (between.match(/<\/div>/g) ?? []).length;
  assert.equal(opened, closed, "role=alert is a sibling of the aria-live=polite wrapper");
});

test("an empty stack lets clicks through and sits above the command palette", () => {
  const html = render(ToastStack).body;
  assert.doesNotMatch(html, /class="[^"]*\bhas-toasts\b/);
  const source = readFileSync(new URL("../src/parts/ToastStack.svelte", import.meta.url), "utf8");
  assert.match(source, /z-index:\s*var\(--z-toast\)/);
  assert.match(source, /max-height:\s*calc\(100vh - 92px\)/);
  assert.match(source, /overflow-y:\s*auto/);
  assert.match(source, /\.toast-stack\.has-toasts\s*\{[^}]*pointer-events:\s*auto/);
  const tokens = readFileSync(new URL("../src/tokens.css", import.meta.url), "utf8");
  assert.match(tokens, /--z-toast:\s*1000;/);
});

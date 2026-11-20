import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { render } from "svelte/server";
import { bridge } from "../src/lib/bridge.svelte.js";
import { bookmarkRows, emptyInspector, historyRows, inspectorHandlers } from "../src/lib/inspector.js";
import InspectorBookmarks from "../src/parts/InspectorBookmarks.svelte";
import InspectorHistory from "../src/parts/InspectorHistory.svelte";

const marks = [
  { time: 23300, label: "Intro", clock: "00:23.300" },
  { time: 45120, label: "", clock: "00:45.120" },
  { time: 61850, label: "Kiai starts", clock: "01:01.850" },
];
const model = (more = {}) => ({ ...emptyInspector, doc: true, bookmarks: marks, bookmark: null,
  history: ["Opened difficulty", "Draw hit", "Move 4 hits"], historyPosition: 1, ...more });

function shown(part, inspector) {
  bridge.inspector = inspector;
  return render(part).body;
}
const buttons = html => [...html.matchAll(/<button\b[^>]*>([^]*?)<\/button>/g)].map(match => ({
  tag: match[0].slice(0, match[0].indexOf(">") + 1), html: match[1], text: match[1].replace(/<[^>]+>/g, " ").replace(/\s+/g, " ").trim(),
}));
const button = (html, text) => buttons(html).find(item => item.text === text);
const label = (html, text) => buttons(html).find(item => new RegExp(`aria-label="${text}"`).test(item.tag));
const disabled = tag => /\sdisabled/.test(tag);
const source = name => readFileSync(new URL(`../src/parts/${name}.svelte`, import.meta.url), "utf8");

test("bookmark rows carry v3's dot, label and time; an unlabelled one is named by its place", () => {
  assert.deepEqual(bookmarkRows(marks, 45120), [
    { time: 23300, label: "Intro", clock: "00:23.300", current: false },
    { time: 45120, label: "Bookmark 2", clock: "00:45.120", current: true },
    { time: 61850, label: "Kiai starts", clock: "01:01.850", current: false },
  ]);
  assert.deepEqual(bookmarkRows([], null), []);
  assert.deepEqual(bookmarkRows(undefined, null), []);
});

test("history rows list the newest step first; steps after the current one can be redone", () => {
  assert.deepEqual(historyRows(["Opened difficulty", "Draw hit", "Move 4 hits"], 1), [
    { label: "Move 4 hits", position: 2, current: false, undone: true },
    { label: "Draw hit", position: 1, current: true, undone: false },
    { label: "Opened difficulty", position: 0, current: false, undone: false },
  ]);
  assert.deepEqual(historyRows([], 0), []);
});

test("Bookmarks show as rows; clicking one picks it and goes to it", () => {
  const html = shown(InspectorBookmarks, model({ bookmark: 61850 }));
  const rows = buttons(html).filter(item => /class="insp-mark\b/.test(item.tag));
  assert.deepEqual(rows.map(row => row.text), ["Intro 00:23.300", "Bookmark 2 00:45.120", "Kiai starts 01:01.850"]);
  for (const row of rows) {
    assert.match(row.html, /^<span class="insp-dot[^"]*" aria-hidden="true"><\/span><span class="insp-mark-label[^"]*"[^>]*>[^<]+<\/span><time class="insp-mark-time[^"]*">/);
    assert.ok(!disabled(row.tag));
  }
  assert.match(rows[2].tag, /aria-current="true"/);
  assert.doesNotMatch(rows[0].tag, /aria-current/);
  assert.match(rows[0].tag, /title="Go to 00:23\.300"/);
  assert.doesNotMatch(html, /No bookmarks/);
});

test("the bookmark list shows six rows, as the old list box did, and scrolls the rest", () => {
  const css = readFileSync(new URL("../src/parts/inspector.css", import.meta.url), "utf8");
  const rule = css.match(/\n\.insp-marks \{([^}]*)\}/)?.[1] ?? "";
  // Six 28 px rows, five 4 px gaps and the 8 px top padding.
  assert.match(rule, /max-height: 196px;/);
  assert.match(rule, /overflow-y: auto;/);
  assert.match(css, /\.insp-mark,\n\.insp-step \{[^}]*height: 28px;[^}]*\}/);
});

test("no bookmarks says how to add one", () => {
  const html = shown(InspectorBookmarks, model({ bookmarks: [] }));
  assert.match(html, />No bookmarks\. Add one at the playhead with \+\.</);
  assert.equal(buttons(html).filter(item => /insp-mark\b/.test(item.tag)).length, 0);
});

// Each control's gate, as the old markup had it: [button, model that enables it, change that disables it].
const gates = [
  ["Add bookmark", {}, { doc: false }],
  ["Previous", {}, { doc: false }],
  ["Next", {}, { doc: false }],
  ["Go to", { bookmark: 45120 }, { bookmark: null }],
  ["Go to", { bookmark: 45120 }, { busy: true }],
  ["Rename…", { bookmark: 45120 }, { bookmark: null }],
  ["Rename…", { bookmark: 45120 }, { busy: true }],
  ["Remove", { bookmark: 45120 }, { bookmark: null }],
  ["Remove", { bookmark: 45120 }, { busy: true }],
];

test("every Bookmarks button waits exactly where the old markup made it wait", () => {
  const find = (html, name) => button(html, name) ?? label(html, name);
  for (const [name, on, off] of gates) {
    assert.ok(!disabled(find(shown(InspectorBookmarks, model(on)), name).tag), name + " enabled");
    assert.ok(disabled(find(shown(InspectorBookmarks, model({ ...on, ...off })), name).tag), name + " disabled by " + JSON.stringify(off));
  }
  // The old list box waited while busy; so do its rows.
  const busy = shown(InspectorBookmarks, model({ busy: true, doc: false }));
  for (const row of buttons(busy).filter(item => /insp-mark\b/.test(item.tag))) assert.ok(disabled(row.tag), row.text);
  // A picked time that is gone is no pick.
  assert.ok(disabled(button(shown(InspectorBookmarks, model({ bookmark: 99 })), "Go to").tag));
});

test("+ sits beside the fold header, not inside it, and stays when the section folds", () => {
  const html = shown(InspectorBookmarks, model());
  const add = label(html, "Add bookmark");
  assert.match(add.tag, /title="Add bookmark at the playhead"/);
  assert.equal(add.text, "");
  const head = buttons(html).find(item => /class="insp-head/.test(item.tag));
  assert.equal(head.text, "Bookmarks");
  assert.match(head.tag, /aria-controls="inspector-bookmarks-body"/);
  assert.ok(html.indexOf('aria-label="Add bookmark"') < html.indexOf('id="inspector-bookmarks-body"'));
});

test("History lists every step, newest first, marks the current one and keeps the scroll id", () => {
  const html = shown(InspectorHistory, model());
  assert.match(html, /<div id="history" class="insp-steps"/);
  const steps = buttons(html).filter(item => /insp-step\b/.test(item.tag));
  assert.deepEqual(steps.map(step => step.text), ["Move 4 hits", "Draw hit", "Opened difficulty"]);
  assert.match(steps[0].tag, /insp-undone/);
  assert.match(steps[1].tag, /aria-current="step"/);
  assert.doesNotMatch(steps[2].tag, /aria-current|insp-undone/);
  // History steps never waited, as before.
  for (const step of buttons(shown(InspectorHistory, model({ busy: true, doc: false }))).filter(item => /insp-step\b/.test(item.tag))) {
    assert.ok(!disabled(step.tag), step.text);
  }
  assert.equal(buttons(shown(InspectorHistory, emptyInspector)).filter(item => /insp-step\b/.test(item.tag)).length, 0);
});

test("both sections fold like the others, with no inline styles or wired selectors", () => {
  for (const [part, title, body] of [[InspectorBookmarks, "Bookmarks", "inspector-bookmarks-body"],
    [InspectorHistory, "History", "inspector-history-body"]]) {
    const html = shown(part, model({ bookmark: 23300 }));
    assert.match(html, new RegExp(`aria-expanded="true" aria-controls="${body}"[^>]*>[^]*?${title}</button>`));
    assert.match(html, new RegExp(`<div id="${body}"`));
    assert.doesNotMatch(html, /\sstyle="/);
    assert.doesNotMatch(html, /data-(action|command|volume|doc|map)\b/);
  }
  for (const name of ["InspectorBookmarks", "InspectorHistory"]) {
    assert.doesNotMatch(source(name), /\{#if open\b/, name);
    assert.match(source(name), /hidden=\{!open\}/, name);
  }
});

test("each Bookmarks and History control sends what app.js's inspectorAction runs", () => {
  const sent = [];
  const on = inspectorHandlers((...args) => sent.push(args));
  on.pickBookmark(45120);
  for (const kind of ["add", "goTo", "rename", "remove"]) on.bookmark(kind);
  on.command("previousBookmark");
  on.command("nextBookmark");
  on.history(0);
  assert.deepEqual(sent, [
    ["pickBookmark", 45120], ["bookmark", "add"], ["bookmark", "goTo"], ["bookmark", "rename"], ["bookmark", "remove"],
    ["command", "previousBookmark"], ["command", "nextBookmark"], ["history", 0],
  ]);
});

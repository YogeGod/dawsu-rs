import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { render } from "svelte/server";
import { bridge } from "../src/lib/bridge.svelte.js";
import { bookmarkRows, cutOffTitle, emptyInspector, historyRows, inspectorHandlers, scrollNearest } from "../src/lib/inspector.js";
import InspectorBookmarks from "../src/parts/InspectorBookmarks.svelte";
import InspectorHistory from "../src/parts/InspectorHistory.svelte";

const marks = [
  { time: 23300, label: "Intro", clock: "00:23.300" },
  { time: 45120, label: "", clock: "00:45.120" },
  { time: 61850, label: "Kiai starts", clock: "01:01.850" },
];
const model = (more = {}) => ({ ...emptyInspector, doc: true, bookmarks: marks, bookmark: null,
  history: ["Opened difficulty", "Draw hit", "Move 4 hits"], historyPosition: 1, ...more });

// Without Svelte's hydration markers (<!--[-->), so the markup reads as the page has it.
function shown(part, inspector) {
  bridge.inspector = inspector;
  return render(part).body.replace(/<!--[^>]*-->/g, "");
}
const buttons = html => [...html.matchAll(/<button\b[^>]*>([^]*?)<\/button>/g)].map(match => ({
  tag: match[0].slice(0, match[0].indexOf(">") + 1), html: match[1], text: match[1].replace(/<[^>]+>/g, " ").replace(/\s+/g, " ").trim(),
}));
const button = (html, text) => buttons(html).find(item => item.text === text);
const label = (html, text) => buttons(html).find(item => new RegExp(`aria-label="${text}"`).test(item.tag));
const disabled = tag => /\sdisabled/.test(tag);
const source = name => readFileSync(new URL(`../src/parts/${name}.svelte`, import.meta.url), "utf8");

test("bookmark rows carry v3's dot, label and time; an unlabelled one shows its time alone", () => {
  assert.deepEqual(bookmarkRows(marks, 45120), [
    { time: 23300, label: "Intro", clock: "00:23.300", current: false },
    { time: 45120, label: "", clock: "00:45.120", current: true },
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
  assert.deepEqual(rows.map(row => row.text), ["Intro 00:23.300", "00:45.120", "Kiai starts 01:01.850"]);
  for (const row of [rows[0], rows[2]]) {
    assert.match(row.html, /^<span class="insp-dot[^"]*" aria-hidden="true"><\/span><span class="insp-mark-label[^"]*">[^<]+<\/span><time class="insp-mark-time[^"]*">/);
  }
  // An unnamed bookmark shows its time alone, as the old list did.
  assert.match(rows[1].html, /^<span class="insp-dot[^"]*" aria-hidden="true"><\/span><time class="insp-mark-time[^"]*">00:45\.120<\/time>$/);
  for (const row of rows) assert.ok(!disabled(row.tag));
  assert.match(rows[2].tag, /aria-current="true"/);
  assert.doesNotMatch(rows[0].tag, /aria-current/);
  // A row has no tooltip until its label is cut off (cutOffTitle sets it on hover).
  for (const row of rows) assert.doesNotMatch(row.tag + row.html, /\stitle=/, row.text);
  assert.doesNotMatch(html, /No bookmarks/);
});

const css = readFileSync(new URL("../src/parts/inspector.css", import.meta.url), "utf8");
const rule = selector => css.match(new RegExp("\\n" + selector.replace(/[.[\]]/g, "\\$&") + " \\{([^}]*)\\}"))?.[1] ?? "";
const px = (body, name) => Number(body.match(new RegExp("(?:^|\\s)" + name + ": (\\d+)px;"))?.[1]);

test("nine bookmarks render nine rows; six show, as the old list box did, and the rest scroll", () => {
  const nine = Array.from({ length: 9 }, (_, index) =>
    ({ time: 1000 * (index + 1), label: "Mark " + (index + 1), clock: "00:0" + (index + 1) + ".000" }));
  const rows = buttons(shown(InspectorBookmarks, model({ bookmarks: nine }))).filter(item => /class="insp-mark\b/.test(item.tag));
  assert.equal(rows.length, 9);
  const list = rule(".insp-marks");
  const shared = rule(".insp-marks,\n.insp-steps");
  const row = rule(".insp-mark,\n.insp-step");
  assert.match(list, /overflow-y: auto;/);
  const [height, top, gap, rowHeight] = [px(list, "max-height"), px(shared, "padding-top"), px(shared, "gap"), px(row, "height")];
  assert.ok([height, top, gap, rowHeight].every(Number.isFinite), "the list's sizes are in px");
  // A row shows in full when the top padding, it and the rows and gaps before it fit the list.
  const visible = rows.filter((_, index) => top + (index + 1) * rowHeight + index * gap <= height).length;
  assert.equal(visible, 6);
});

test("the picked row and the current History step keep their fill on hover", () => {
  const hover = css.indexOf("\n.insp-mark:hover:not(:disabled),\n.insp-step:hover:not(:disabled) {");
  const kept = css.indexOf("\n.insp-mark[aria-current]:hover,\n.insp-step[aria-current]:hover {");
  assert.ok(hover >= 0, "the hover rule");
  assert.ok(kept >= 0, "the [aria-current]:hover rule");
  assert.match(rule(".insp-mark[aria-current]:hover,\n.insp-step[aria-current]:hover"), /background: var\(--editor-rowfocus\);/);
  // Both rules weigh the same, so the kept fill has to come later.
  assert.ok(kept > hover);
});

test("no bookmarks says how to add one", () => {
  const html = shown(InspectorBookmarks, model({ bookmarks: [] }));
  assert.match(html, />No bookmarks\. Add one at the playhead with \+\.</);
  assert.equal(buttons(html).filter(item => /insp-mark\b/.test(item.tag)).length, 0);
});

// Each control's gate, as the old markup had it: [button, model that enables it, change that disables it].
const gates = [
  ["Add bookmark", {}, { doc: false }],
  ["Add bookmark", {}, { busy: true }],
  ["Previous", {}, { doc: false }],
  ["Previous", {}, { busy: true }],
  ["Next", {}, { doc: false }],
  ["Next", {}, { busy: true }],
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
  assert.match(add.tag, /title="Add bookmark at the playhead \(Ctrl\+B\)"/);
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
  for (const [part, title, body, initial] of [[InspectorBookmarks, "Bookmarks", "inspector-bookmarks-body", "true"],
    [InspectorHistory, "History", "inspector-history-body", "false"]]) {
    const html = shown(part, model({ bookmark: 23300 }));
    assert.match(html, new RegExp(`aria-expanded="${initial}" aria-controls="${body}"[^>]*>[^]*?${title}</button>`));
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

test("a row's tooltip names the bookmark only while its label is cut off", () => {
  const row = (scrollWidth, clientWidth) => ({
    title: "stale", querySelector: selector => (selector === ".insp-mark-label" ? { scrollWidth, clientWidth } : null),
    removeAttribute(name) { if (name === "title") this.title = ""; },
  });
  const cut = row(180, 120);
  cutOffTitle(cut, "A very long bookmark label");
  assert.equal(cut.title, "A very long bookmark label");
  const whole = row(80, 120);
  cutOffTitle(whole, "Intro");
  assert.equal(whole.title, "");
  // An unnamed row shows its whole time and has no label to cut.
  const unnamed = { title: "stale", querySelector: () => null, removeAttribute() { this.title = ""; } };
  cutOffTitle(unnamed, "");
  assert.equal(unnamed.title, "");
  assert.match(source("InspectorBookmarks"), /onpointerenter=\{event => cutOffTitle\(event\.currentTarget, row\.label\)\}/);
});

test("History brings its current step into view the nearest way, moving only its own list", () => {
  const list = (top, bottom, scrollTop) => ({ scrollTop, getClientRects: () => [{}], getBoundingClientRect: () => ({ top, bottom }) });
  const step = (top, bottom) => ({ getBoundingClientRect: () => ({ top, bottom }) });
  const above = list(100, 330, 200);
  scrollNearest(above, step(60, 88));
  assert.equal(above.scrollTop, 160);
  const below = list(100, 330, 0);
  scrollNearest(below, step(320, 348));
  assert.equal(below.scrollTop, 18);
  const inside = list(100, 330, 40);
  scrollNearest(inside, step(150, 178));
  assert.equal(inside.scrollTop, 40);
  scrollNearest(inside, null);
  assert.equal(inside.scrollTop, 40);
  // A folded section has no box, so nothing moves.
  const folded = { scrollTop: 5, getClientRects: () => [], getBoundingClientRect: () => ({ top: 0, bottom: 0 }) };
  scrollNearest(folded, step(320, 348));
  assert.equal(folded.scrollTop, 5);
  // The part runs it each time the position changes; scrollIntoView would move the whole Inspector too.
  assert.match(source("InspectorHistory"), /\$effect\(\(\) => \{[^}]*position[^}]*scrollNearest\(/);
  assert.doesNotMatch(source("InspectorHistory"), /scrollIntoView/);
});

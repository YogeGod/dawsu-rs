import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";

const read = name => readFile(new URL("../dist/" + name, import.meta.url), "utf8");
const [app, html, css] = await Promise.all(["app.js", "index.html", "style.css"].map(read));
const inspector = html.match(/<aside id="editor-inspector"[^>]*>([\s\S]*?)<\/aside>/)?.[1] ?? "";

async function frontend() {
  const controls = new Map();
  function element() {
    return {
      events: new Map(), children: [], dataset: {}, attributes: {},
      style: { setProperty(name, value) { this[name] = value; } }, classList: { add() {}, toggle() {} },
      addEventListener(name, listener) { this.events.set(name, listener); },
      setAttribute(name, value) { this.attributes[name] = String(value); },
      append(...children) { this.children.push(...children); },
      prepend() {}, focus() {},
      replaceChildren(...children) { this.children = children; },
    };
  }
  const get = id => {
    if (!controls.has(id)) controls.set(id, element());
    return controls.get(id);
  };
  const context = createContext({
    document: { getElementById: get, createElement: element, querySelectorAll: () => [], addEventListener() {} },
    window: { __TAURI__: {
      core: { invoke: async () => ({ busy: false, hasDocument: true, position: 0 }) },
      event: { listen() {} },
    } },
    ResizeObserver: class { observe() {} },
    Option: class { constructor(text, value) { this.text = text; this.value = value; } },
    requestAnimationFrame() {},
    setInterval() {},
    console,
  });
  await runInContext(app, context);
  return { evaluate: code => runInContext(code, context), get };
}

test("each Inspector section folds under a plain heading and keeps every control", () => {
  const headings = [...inspector.matchAll(/<section[^>]*>\s*<details open>\s*<summary><h2>([^<]*)<\/h2><\/summary>/g)]
    .map(match => match[1]);
  assert.deepEqual(headings, ["Selection", "Lane", "Difficulty", "Bookmarks", "History"]);
  // The layout editor turns every Inspector <button> into a movable button, so headings hold none.
  for (const [, summary] of inspector.matchAll(/<summary>([\s\S]*?)<\/summary>/g)) assert.doesNotMatch(summary, /<button/);
  for (const id of [
    "selected", "selection-fields", "selection-start-edit", "selection-start-input", "selection-object",
    "selection-volume", "volume-mode", "volume", "selection-volume-scope", "selection-volume-hint", "edit-range",
    "slider-body-fields", "slider-body-set", "slider-body-whistle", "slider-body-whistle-set", "slider-body-volume",
    "apply-slider-body", "insert", "protect", "audition", "active-lane-name", "active-lane-sample",
    "active-lane-colour-swatch", "replace-sample", "lane-colour", "hs", "generate", "new", "copy", "merge-settings",
    "add-bookmark", "bookmarks", "jump-bookmark", "rename-bookmark", "remove-bookmark", "classic-browser-host", "history",
  ]) assert.match(inspector, new RegExp(`id="${id}"`), id);
  for (const command of ["quantizeDialog", "repeat", "duplicate", "delete", "editRange", "scaleByVolume",
    "previousBookmark", "nextBookmark"]) assert.match(inspector, new RegExp(`data-command="${command}"`), command);
  assert.match(inspector, /data-volume="-5"/);
  assert.match(inspector, /data-volume="5"/);
  assert.match(css, /#editor-inspector summary::-webkit-details-marker\s*\{\s*display:\s*none/);
  // Rules for the old Inspector markup are gone with it.
  for (const dead of [".selection-value", ".active-lane-identity", ".active-lane-actions"]) assert.ok(!css.includes(dead), dead);
});

test("Selection shows Start, Volume and On object as rows with the selected lanes", async () => {
  const { evaluate, get } = await frontend();
  evaluate(`
    view = { busy: false, hasDocument: true };
    scene = { path: "a.osu",
      lanes: [{ key: "k", name: "Kick", sample: "kick.wav" }, { key: "c", name: "Clap" }],
      moments: [[1000, 80, true, [[0, true], [1, false]]], [1500, 80, true, [[0, true, false, 60]]]],
      selection: { count: 2, start: 1000, object: "2 object moments" } };
    renderSelection();
  `);
  assert.equal(get("selected").textContent, "2 hits");
  assert.equal(get("selected-lanes").textContent, " · Kick");
  assert.equal(get("selection-start-input").value, "00:01.000");
  assert.equal(get("selection-volume-value").textContent, "60–80%");
  assert.equal(get("selection-object").textContent, "2 object moments");

  evaluate(`scene = { ...scene, moments: [[1000, 80, true, [[0, true], [1, true]]]],
    selection: { count: 2, start: 1000, object: "Circle" } }; renderSelection();`);
  assert.equal(get("selected-lanes").textContent, " · 2 lanes");
  assert.equal(get("selection-volume-value").textContent, "80%");

  evaluate("scene = { lanes: [], selection: { count: 1, start: 5, object: \"No object\" } }; renderSelection();");
  assert.equal(get("selected-lanes").textContent, "");
  assert.equal(get("selection-volume-value").textContent, "");

  evaluate("scene = { selection: null }; renderSelection();");
  assert.equal(get("selected").textContent, "Nothing selected");
  assert.equal(get("selected-lanes").textContent, "");
});

test("Lane shows the active lane's tint as a row, from the sound colour until the mapper picks one", async () => {
  const { evaluate, get } = await frontend();
  evaluate(`
    view = { busy: false, hasDocument: true, activeLane: 0 };
    scene = { path: "a.osu", lanes: [{ key: "k", name: "Kick", sample: "kick.wav", colourKey: "File|kick.wav", canReplace: true }] };
    renderActiveLane();
  `);
  assert.equal(get("active-lane-name").textContent, "Kick");
  assert.equal(get("active-lane-colour-swatch").hidden, false);
  assert.equal(get("active-lane-colour-swatch").style.backgroundColor, "#62d6cf");
  assert.equal(get("active-lane-colour-name").textContent, "Sound colour");
  assert.equal(get("lane-colour").ariaLabel, "Lane colour: Sound colour");
  assert.equal(get("lane-colour").disabled, false);
  evaluate('laneColours.set("File|kick.wav", "#cc50a2"); renderActiveLane();');
  assert.equal(get("active-lane-colour-swatch").style.backgroundColor, "#cc50a2");
  assert.equal(get("active-lane-colour-name").textContent, "#cc50a2");
  assert.equal(get("lane-colour").ariaLabel, "Lane colour: #cc50a2");
  evaluate("view.activeLane = null; renderActiveLane();");
  assert.equal(get("active-lane-colour-swatch").hidden, true);
  assert.equal(get("active-lane-colour-name").textContent, "");
  assert.equal(get("lane-colour").ariaLabel, "Lane colour…");
});

test("Bookmarks list as rows sized to the marks and say when there are none", async () => {
  const { evaluate, get } = await frontend();
  evaluate(`view = { busy: false }; scene = { path: "a.osu", bookmarks: [
    { time: 500, label: "Intro" }, { time: 1000, label: "" }, { time: 2000, label: "Kiai" }] }; renderBookmarks();`);
  assert.equal(get("bookmarks").size, 3);
  assert.equal(get("bookmarks").hidden, false);
  assert.equal(get("bookmarks-empty").hidden, true);
  evaluate(`scene = { path: "b.osu", bookmarks: Array.from({ length: 9 }, (_, i) => ({ time: i * 100, label: "" })) };
    renderBookmarks();`);
  assert.equal(get("bookmarks").size, 6);
  evaluate('scene = { path: "c.osu", bookmarks: [] }; renderBookmarks();');
  assert.equal(get("bookmarks").hidden, true);
  assert.equal(get("bookmarks-empty").hidden, false);
});

test("Start shows and takes osu! editor times, and plain milliseconds", async () => {
  const { evaluate, get } = await frontend();
  const calls = [];
  evaluate(`view = { busy: false, hasDocument: true };
    scene = { path: "a.osu", lanes: [], moments: [], selection: { count: 1, start: 105263, object: "Circle" } };
    renderSelection();`);
  const field = get("selection-start-input");
  assert.equal(field.value, "01:45.263");
  field.blur = () => {};
  evaluate("act = action => { globalThis.moved = action.edit.position; };");
  const move = text => {
    evaluate("globalThis.moved = null;");
    field.value = text;
    get("selection-start-edit").events.get("submit")({ preventDefault() {} });
    return evaluate("globalThis.moved");
  };
  assert.equal(move("01:45.263"), null, "the time already shown moves nothing");
  assert.equal(move("01:46.5"), 106500);
  assert.equal(move("1:05:020 (1,2) -"), 65020, "a modding timestamp");
  assert.equal(move("1330.5"), 1330.5);
  assert.equal(move(" 2000 "), 2000);
  assert.equal(move("soon"), null);
  assert.equal(move("12:3x"), null);
});

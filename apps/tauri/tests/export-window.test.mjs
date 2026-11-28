import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";
import { copyOptions, defaultChoices } from "../ui/src/lib/export.js";
import { bootEditor } from "./editor-runtime.mjs";

const tauri = new URL("../", import.meta.url);
const read = path => readFileSync(new URL(path, tauri), "utf8");

function editor() {
  const calls = [];
  const makeElement = () => ({
    events: new Map(), children: [],
    addEventListener(name, callback) { this.events.set(name, callback); },
    append(...children) { this.children.push(...children); },
    setAttribute() {},
    prepend(child) { this.children.unshift(child); },
    get valueAsNumber() { return this.value === "" ? NaN : Number(this.value); },
  });
  const elements = new Map();
  const get = id => {
    if (!elements.has(id)) elements.set(id, makeElement());
    return elements.get(id);
  };
  const context = createContext({
    document: { getElementById: get, createElement: makeElement, querySelectorAll: () => [], addEventListener() {} },
    window: { __TAURI__: {
      core: { invoke: async (name, args) => {
        calls.push({ name, args });
        return { busy: false, hasDocument: false };
      } },
      event: { listen() {} },
    } },
    ResizeObserver: class { observe() {} },
    setInterval() {},
    requestAnimationFrame() {},
  });
  bootEditor(context);
  return { calls, get, context };
}

test("Export… in the top bar opens the Export window", async () => {
  const { calls, get } = editor();
  await new Promise(done => setTimeout(done));
  calls.length = 0;
  get("export-entry").events.get("click")();
  assert.deepEqual(calls.map(call => call.name), ["open_export_window"], "at once, not after the editor's queue");
});

test("Export copies with exactly the options the old Copy dialog sends by default", async () => {
  const { calls, get, context } = editor();
  await new Promise(done => setTimeout(done));
  runInContext(
    'scene = { path: "Source.osu", difficulties: ["Source.osu", "Target.osu"] };' +
    'modal = (_kind, _title, content, buttons) => { globalThis.copy = { content, buttons }; };' +
    'closeModal = () => {};', context,
  );
  runInContext("inspectorAction('copy')", context);
  function* descendants(nodes) {
    for (const node of nodes) {
      if (typeof node === "string") continue;
      yield node;
      yield* descendants(node.children);
    }
  }
  const target = Array.from(descendants(context.copy.content))
    .find(node => node.children[0] === "Target.osu").children[1];
  target.checked = true;
  calls.length = 0;
  context.copy.buttons.find(([label]) => label === "Review output")[1]();
  await runInContext("pending", context);
  const sent = JSON.parse(JSON.stringify(calls[0].args.action));
  assert.equal(sent.kind, "copy");
  assert.deepEqual(copyOptions(defaultChoices), sent.options);
});

// The editor with a map open whose answers the test sets field by field.
async function answering() {
  const calls = [];
  const status = { busy: false, hasDocument: false, duration: 10_000, playing: false, position: 0 };
  const makeElement = () => ({
    events: new Map(), children: [], dataset: {}, style: { setProperty() {} },
    addEventListener(name, callback) { this.events.set(name, callback); },
    append() {}, prepend() {}, setAttribute() {},
    getBoundingClientRect: () => ({ x: 0, top: 0, width: 1000 }),
  });
  const elements = new Map();
  const get = id => {
    if (!elements.has(id)) elements.set(id, makeElement());
    return elements.get(id);
  };
  const toasts = [];
  const context = createContext({
    document: { documentElement: { dataset: {} }, getElementById: get, createElement: makeElement,
      querySelectorAll: () => [], addEventListener() {} },
    window: {
      __TAURI__: {
        core: { invoke: async (name, args) => { calls.push({ name, args }); return { ...status }; } },
        event: { listen() {} },
      },
      showToast: toast => toasts.push(toast),
      openExportModal: () => {},
    },
    performance: { now: () => 0 },
    ResizeObserver: class { observe() {} },
    HTMLElement: class {},
    requestAnimationFrame() {},
    setInterval() {},
    console,
  });
  const ctx = bootEditor(context, { autoStart: false });
  await ctx.act(null);
  status.hasDocument = true;
  runInContext('view = { busy: false, hasDocument: true, duration: 10_000 }; scene = { path: "fixture.osu", viewStart: 0, zoom: 1, rowHeight: 30 }', context);
  const answer = async fields => { Object.assign(status, fields); await ctx.act(null); };
  return { answer, calls, ctx, get, toasts };
}

test("a hitsound-only Export says so with Open folder, once per write, also for the same write again", async () => {
  const { answer, calls, ctx, get, toasts } = await answering();
  const written = "Exported 2 hitsound-only file(s) to C:\\Exports\\Set-hitsound-only (mapset untouched).";
  const titles = () => toasts.map(toast => toast.title);
  get("export-entry").events.get("click")();
  await answer({ status: written });
  assert.deepEqual(titles(), ["Hitsound-only export written"]);
  assert.equal(toasts[0].body, written);
  assert.deepEqual(toasts[0].actions.map(action => action.label), ["Open folder"]);
  calls.length = 0;
  toasts[0].actions[0].onClick();
  assert.deepEqual(calls, [{ name: "open_editor_folder", args: { folder: "last_export" } }]);
  // The modal closing on that write asks for a view, and the write is not told twice.
  await ctx.exportClosed(true);
  assert.equal(toasts.length, 1);
  // Opened again, views before the next write tell nothing.
  get("export-entry").events.get("click")();
  await answer({});
  await ctx.exportClosed(false);
  assert.equal(toasts.length, 1, "Cancel writes nothing");
  // The identical write to the same folder: its view looks the same, the modal's close tells it.
  get("export-entry").events.get("click")();
  await answer({ status: written });
  assert.equal(toasts.length, 1);
  await ctx.exportClosed(true);
  assert.deepEqual(titles(), ["Hitsound-only export written", "Hitsound-only export written"]);
  assert.equal(toasts[1].actions.length, 1);
});

test("a hitsound-only Export is still told when a stem message replaced the Exported status", async () => {
  for (const stemStatus of ["Loading drum stem…", "Drum stem ready"]) {
    const { answer, calls, ctx, get, toasts } = await answering();
    get("export-entry").events.get("click")();
    // The write finished, but a background stem load set the status before the editor saw it.
    await answer({ status: stemStatus });
    assert.equal(toasts.length, 0);
    await ctx.exportClosed(true);
    assert.deepEqual(toasts.map(toast => toast.title), ["Hitsound-only export written"], stemStatus);
    assert.match(toasts[0].body, /hitsound-only/);
    assert.deepEqual(toasts[0].actions.map(action => action.label), ["Open folder"]);
    calls.length = 0;
    toasts[0].actions[0].onClick();
    assert.deepEqual(calls, [{ name: "open_editor_folder", args: { folder: "last_export" } }]);
    // Closing again without a new write tells nothing more.
    await ctx.exportClosed(false);
    assert.equal(toasts.length, 1);
  }
});

test("a Save told while the Export modal is open does not hide its hitsound-only write", async () => {
  for (const stemStatus of ["Loading drum stem…", "Drum stem ready"]) {
    const { answer, ctx, get, toasts } = await answering();
    await answer({ dirty: true });
    get("export-entry").events.get("click")();
    // Ctrl+S while the modal is open: the Saved toast is told.
    await answer({ dirty: false, status: "Saved Target.osu." });
    assert.deepEqual(toasts.map(toast => toast.title), ["Saved"]);
    // The hitsound-only write (id 1) finishes, then a stem message replaces its status.
    await answer({ status: stemStatus });
    await ctx.exportClosed(true, 1);
    assert.deepEqual(toasts.map(toast => toast.title), ["Saved", "Hitsound-only export written"], stemStatus);
    assert.deepEqual(toasts[1].actions.map(action => action.label), ["Open folder"]);
  }
});

test("a late Exported view after the modal told its write gives no second toast", async () => {
  const { answer, ctx, get, toasts } = await answering();
  const written = "Exported 2 hitsound-only file(s) to C:\\Exports\\Set-hitsound-only (mapset untouched).";
  get("export-entry").events.get("click")();
  await answer({ status: "Drum stem ready" });
  await ctx.exportClosed(true, 1);
  assert.equal(toasts.length, 1);
  // The Exported status of that same write shows up only now.
  await answer({ status: written });
  assert.equal(toasts.length, 1, "write 1 is told once");
  // A new Export session's write is still told.
  get("export-entry").events.get("click")();
  await answer({ status: written.replace("Set-hitsound-only", "Other-hitsound-only") });
  assert.equal(toasts.length, 2);
  await ctx.exportClosed(true, 2);
  assert.equal(toasts.length, 2, "write 2 is told once");
});

test("export.html loads only the fonts and the Svelte bundle, with no inline code", () => {
  const html = read("dist/export.html");
  const head = html.match(/<head>(.*?)<\/head>/s)[1];
  const order = [...head.matchAll(/(?:href|src)="([^"]+\.(?:css|js))"/g)].map(match => match[1]);
  assert.deepEqual(order, ["fonts.css", "ui/ui.css", "ui/ui.js"]);
  assert.match(head, /<script defer src="ui\/ui\.js"><\/script>/);
  assert.doesNotMatch(html, /<script(?![^>]*\bsrc=)[^>]*>/, "no inline script");
  assert.doesNotMatch(html, /<style|\sstyle=|\son\w+=/, "no inline style or handler");
  assert.equal([...html.matchAll(/data-ui-mount="([^"]+)"/g)].map(match => match[1]).join(), "export-window");
  assert.match(html, /<body class="export-page">/);
});

test("fonts.css holds the Inter face that both windows load", () => {
  assert.match(read("dist/fonts.css"), /@font-face\s*\{\s*font-family:\s*Inter;/);
  assert.doesNotMatch(read("ui/src/legacy-chrome.css"), /Inter-Variable/);
});

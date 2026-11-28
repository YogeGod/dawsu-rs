import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";
import { bootEditor } from "./editor-runtime.mjs";
import { initMenus } from "../ui/src/editor/menus.js";
import * as commandList from "../ui/src/lib/commands.js";
import { hasOpenModal, onlyModalOpen } from "../ui/src/lib/registry.js";

// The dialog box sits at (100,100)-(500,400); its padding is inside that box.
const DIALOG_BOX = { left: 100, top: 100, right: 500, bottom: 400, width: 400, height: 300, x: 100, y: 100 };

function element(tag = "div") {
  const el = {
    tagName: tag.toUpperCase(), events: new Map(), dataset: {}, children: [], open: false,
    className: "", textContent: "", style: {}, attributes: {},
    classList: {
      add(name) { if (!el.classList.contains(name)) el.className = (el.className + " " + name).trim(); },
      remove(name) { el.className = el.className.split(/\s+/).filter(one => one !== name).join(" "); },
      contains(name) { return el.className.split(/\s+/).includes(name); },
      toggle() {},
    },
    append() {}, prepend() {},
    setAttribute(name, value) { el[name] = value; el.attributes[name] = value; },
    removeAttribute(name) { delete el.attributes[name]; },
    getAttribute(name) { return el.attributes[name] ?? null; },
    contains() { return false; },
    focus() {},
    replaceChildren(...children) { el.children = children; },
    addEventListener(name, handler) {
      if (!el.events.has(name)) el.events.set(name, []);
      el.events.get(name).push(handler);
    },
    dispatch(name, event = {}) {
      for (const handler of el.events.get(name) ?? []) handler({ target: el, currentTarget: el, preventDefault() {}, ...event });
    },
    showModal() { el.open = true; },
    close() { el.open = false; },
    getBoundingClientRect: () => DIALOG_BOX,
  };
  return el;
}

async function launch(invoke = async () => ({ busy: false, hasDocument: false })) {
  const elements = new Map();
  const document = {
    getElementById(id) {
      if (!elements.has(id)) elements.set(id, element(id === "dialog" ? "dialog" : "div"));
      return elements.get(id);
    },
    createElement: element,
    querySelectorAll: () => [],
    addEventListener() {},
  };
  const context = createContext({
    document,
    window: { __TAURI__: {
      core: { invoke },
      event: { listen() {} },
    } },
    ResizeObserver: class { observe() {} },
    setInterval() {},
    requestAnimationFrame() {},
  });
  await bootEditor(context);
  const dialog = document.getElementById("dialog");
  return {
    context,
    element: id => document.getElementById(id),
    dialog,
    run: source => runInContext(source, context),
    title: () => document.getElementById("dialog-title").textContent,
    buttons: () => document.getElementById("dialog-actions").children,
    button: label => document.getElementById("dialog-actions").children.find(one => one.textContent === label),
  };
}

test("confirm answers true from its confirm button and closes", async () => {
  const app = await launch();
  const answer = app.run('confirm({ title: "Delete model?", body: "It frees 166 MB.", confirmLabel: "Delete" })');
  assert.equal(app.dialog.open, true);
  assert.equal(app.title(), "Delete model?");
  assert.deepEqual(app.buttons().map(one => one.textContent), ["Cancel", "Delete"]);
  assert.equal(hasOpenModal(), true);
  assert.equal(onlyModalOpen("shared-base"), false);
  app.button("Delete").dispatch("click");
  assert.equal(await answer, true);
  assert.equal(app.dialog.open, false);
  assert.equal(hasOpenModal(), false);
});

test("confirm answers false from Cancel", async () => {
  const app = await launch();
  const answer = app.run('confirm({ title: "Overwrite?" })');
  app.button("Cancel").dispatch("click");
  assert.equal(await answer, false);
  assert.equal(app.dialog.open, false);
  assert.equal(hasOpenModal(), false);
});

test("confirm answers false on Escape", async () => {
  const app = await launch();
  const answer = app.run('confirm({ title: "Overwrite?" })');
  app.dialog.dispatch("cancel");
  assert.equal(await answer, false);
  assert.equal(app.dialog.open, false);
  assert.equal(hasOpenModal(), false);
});

test("a click on the backdrop answers false, a click in the dialog padding does not", async () => {
  const app = await launch();
  const answer = app.run('confirm({ title: "Overwrite?" })');
  app.dialog.dispatch("click", { clientX: 110, clientY: 110 });
  assert.equal(app.dialog.open, true, "padding click keeps the dialog open");
  app.dialog.dispatch("click", { clientX: 40, clientY: 250 });
  assert.equal(await answer, false);
  assert.equal(app.dialog.open, false);
});

test("a click on a dialog button never counts as a backdrop click", async () => {
  const app = await launch();
  const answer = app.run('confirm({ title: "Overwrite?" })');
  // Keyboard-activated clicks report (0,0); they come from a button, not the dialog itself.
  app.dialog.dispatch("click", { target: app.button("Cancel"), clientX: 0, clientY: 0 });
  assert.equal(app.dialog.open, true);
  app.button("OK").dispatch("click");
  assert.equal(await answer, true);
});

test("another dialog replacing confirm answers false", async () => {
  const app = await launch();
  const answer = app.run('confirm({ title: "Overwrite?" })');
  app.run('modal("error", "Could not save", [], [["OK", () => closeModal(), true]])');
  assert.equal(await answer, false);
  assert.equal(app.dialog.open, true);
  assert.equal(app.title(), "Could not save");
  assert.equal(hasOpenModal(), false);
  app.run("closeModal()");
});

test("closeModal answers an open confirm with false", async () => {
  const app = await launch();
  const answer = app.run('confirm({ title: "Overwrite?" })');
  app.run("closeModal()");
  assert.equal(await answer, false);
  assert.equal(hasOpenModal(), false);
});

test("a second confirm answers the first with false and takes its place", async () => {
  const app = await launch();
  const first = app.run('confirm({ title: "First?" })');
  const second = app.run('confirm({ title: "Second?", confirmLabel: "Yes" })');
  assert.equal(await first, false);
  assert.equal(app.dialog.open, true);
  assert.equal(app.title(), "Second?");
  assert.equal(hasOpenModal(), true);
  app.button("Yes").dispatch("click");
  assert.equal(await second, true);
  assert.equal(hasOpenModal(), false);
});

test("danger marks the confirm button red", async () => {
  const app = await launch();
  const answer = app.run('confirm({ title: "Delete?", confirmLabel: "Delete", danger: true })');
  assert.equal(app.button("Delete").classList.contains("danger"), true);
  assert.equal(app.button("Cancel").classList.contains("danger"), false);
  app.button("Cancel").dispatch("click");
  await answer;
  const plain = app.run('confirm({ title: "Overwrite?" })');
  assert.equal(app.button("OK").classList.contains("danger"), false);
  app.button("Cancel").dispatch("click");
  await plain;
  const css = readFileSync(new URL("../ui/src/legacy-chrome.css", import.meta.url), "utf8");
  assert.match(css, /#dialog-actions button\.danger\{[^}]*background:/);
});

test("Shared Base and Stem separation leave Escape to a confirm open above them", () => {
  for (const [file, id] of [["SharedBaseModal.svelte", "shared-base"], ["StemSeparationModal.svelte", "stem-separation"]]) {
    const source = readFileSync(new URL("../ui/src/parts/" + file, import.meta.url), "utf8");
    const handler = source.slice(source.indexOf("function onKeydown"), source.indexOf("$effect", source.indexOf("function onKeydown")));
    assert.match(handler, new RegExp(`event\\.key === "Escape" && bridge\\.onlyModalOpen\\("${id}"\\)`), file);
  }
});

test("native dialog close event resolves confirm to false and clears modal state", async () => {
  const app = await launch();
  const answer = app.run('confirm({ title: "Overwrite?" })');
  assert.equal(app.dialog.open, true);
  assert.equal(hasOpenModal(), true);
  app.dialog.close();
  app.dialog.dispatch("close");
  assert.equal(await answer, false);
  assert.equal(hasOpenModal(), false);
  assert.equal(app.run("modalKind"), "");
});

test("delayed close event from a prior dialog does not cancel a newly opened confirm", async () => {
  const app = await launch();
  const first = app.run('confirm({ title: "First?" })');
  const second = app.run('confirm({ title: "Second?", confirmLabel: "Yes" })');
  assert.equal(await first, false);
  assert.equal(app.dialog.open, true);
  // Asynchronous native "close" from an earlier close arrives while the new confirm is open.
  app.dialog.dispatch("close");
  assert.equal(await Promise.race([second, Promise.resolve("pending")]), "pending");
  assert.equal(hasOpenModal(), true);
  assert.equal(app.run("modalKind"), "confirm");
  app.button("Yes").dispatch("click");
  assert.equal(await second, true);
  assert.equal(hasOpenModal(), false);
});

test("the difficulty list's Capture asks before replacing a Shared Base, and not before the first one", async () => {
  const settle = async () => { for (let i = 0; i < 5; i++) await new Promise(done => setTimeout(done, 0)); };
  for (const hasBase of [true, false]) {
    const kinds = [];
    const app = await launch(async (name, args) => {
      if (name !== "shared_base_action") return { busy: false, hasDocument: false };
      kinds.push(args.action.kind);
      return { hasBase, enabled: hasBase, sourceDifficulty: "Insane", baseMomentCount: 9 };
    });
    let bumps = 0;
    app.context.bumpSharedBaseRev = () => { bumps += 1; };
    app.run('scene = { path: "Test.osu" }');
    const select = app.element("difficulty");
    select.value = "__capture_shared_base__";
    select.dispatch("change");
    await settle();
    if (hasBase) {
      assert.equal(app.dialog.open, true, "a Base exists: Capture asks first");
      assert.equal(app.title(), "Replace the Shared Base?");
      app.button("Cancel").dispatch("click");
      await settle();
      assert.deepEqual(kinds, ["status"], "Cancel sends no capture");
      assert.equal(bumps, 0);
    } else {
      assert.equal(app.dialog.open, false, "a first Capture asks nothing");
      assert.deepEqual(kinds, ["status", "capture"]);
      assert.equal(bumps, 1, "the strip reloads");
    }
  }
});

test("palette Shared Base commands settle the real editor run() queue without deadlocking", async () => {
  const settle = async () => { for (let i = 0; i < 5; i++) await new Promise(done => setTimeout(done, 0)); };
  const within = (promise, label) => Promise.race([
    promise,
    new Promise((_, reject) => setTimeout(() => reject(new Error(`${label} timed out (run queue deadlocked)`)), 500)),
  ]);
  const editorViews = [];
  const sharedBaseKinds = [];
  const toasts = [];
  let bumps = 0;

  const app = await launch(async (name, args) => {
    if (name === "editor_view") {
      editorViews.push({ scene: args?.scene, action: args?.action });
      return { busy: false, hasDocument: true, sharedBaseEnabled: true };
    }
    if (name === "shared_base_action") {
      sharedBaseKinds.push(args.action.kind);
      if (args.action.kind === "status") return { hasBase: false, enabled: false, baseMomentCount: 0 };
      if (args.action.kind === "apply_all") {
        return { updatedDiffs: 1, totalAppliedMoments: 4, totalSkippedOverrides: 0, failedDiffs: ["Hard: could not read"] };
      }
      return { hasBase: true, enabled: true, baseMomentCount: 4 };
    }
    return { busy: false, hasDocument: false };
  });

  app.context.window.addEventListener = () => {};
  app.context.window.showToast = toast => toasts.push(toast);
  app.context.window.dawsuUi = { commands: commandList };
  app.context.bumpSharedBaseRev = () => { bumps += 1; };
  app.context.openSharedBase = () => {};
  app.run('view = { busy: false, hasDocument: true, sharedBaseEnabled: true }; scene = { path: "Test.osu", difficulties: ["Test.osu", "Test [Hard].osu"], lanes: [], moments: [] };');
  editorViews.length = 0;

  const { paletteMenu } = initMenus(app.context, { ui: app.context.window.dawsuUi });
  const runItem = id => paletteMenu.runItem(commandList.paletteOnly.find(item => item.id === id));
  const waitUntilDone = async (check, label) => {
    const deadline = Date.now() + 500;
    while (!check()) {
      if (Date.now() > deadline) throw new Error(`${label} timed out (run queue deadlocked)`);
      await new Promise(done => setTimeout(done, 5));
    }
    await within(app.run("pending"), `${label} pending`);
  };

  runItem("paletteApplySharedBaseAll");
  await settle();
  assert.equal(app.dialog.open, true);
  app.button("Apply to all").dispatch("click");
  await waitUntilDone(() => bumps === 1 && app.run("requests") === 0, "paletteApplySharedBaseAll");
  assert.equal(toasts.at(-1).tone, "error");
  assert.match(toasts.at(-1).body, /Hard: could not read/);

  runItem("paletteToggleSharedBase");
  await waitUntilDone(() => bumps === 2 && app.run("requests") === 0, "paletteToggleSharedBase");

  runItem("paletteCaptureSharedBase");
  await waitUntilDone(() => bumps === 3 && app.run("requests") === 0, "paletteCaptureSharedBase");
  assert.equal(toasts.at(-1).tone, "success");

  assert.deepEqual(sharedBaseKinds, ["apply_all", "set_enabled", "status", "capture"]);
  assert.deepEqual(editorViews, [
    { scene: true, action: null },
    { scene: false, action: null },
    { scene: false, action: null },
  ]);

  await within(app.run("act(null, false)"), "follow-up act");
  assert.equal(app.run("requests"), 0);
});

test("Open from osu! opens the map osu! has in its editor instead of deadlocking the run() queue", async () => {
  // openFromOsu awaited ctx.act() inside ctx.run(): act queues a second run() behind the first, so the
  // open never happened, no error showed, and every later action waited forever.
  const opened = [];
  const app = await launch(async (name, args) => {
    if (name === "open_from_osu") return "F:/osu!/Songs/1 A - B/A - B (C) [D].osu";
    if (name === "editor_view") {
      if (args?.action?.kind === "open") opened.push(args.action.path);
      return { busy: false, hasDocument: opened.length > 0 };
    }
    return { busy: false, hasDocument: false };
  });
  app.context.window.addEventListener = () => {};
  const done = app.run("openFromOsu()");
  await Promise.race([done, new Promise((_, reject) => setTimeout(() => reject(new Error("openFromOsu timed out (run queue deadlocked)")), 500))]);
  assert.deepEqual(opened, ["F:/osu!/Songs/1 A - B/A - B (C) [D].osu"]);
  assert.equal(app.run("requests"), 0);
});

test("Create [Hitsounds] from current difficulty opens the new diff instead of deadlocking the run() queue", async () => {
  const opened = [];
  const app = await launch(async (name, args) => {
    if (name === "create_hitsound_diff_from_current") return "F:/Songs/x/A - B (C) [Hitsounds].osu";
    if (name === "editor_view") {
      if (args?.action?.kind === "open") opened.push(args.action.path);
      return { busy: false, hasDocument: true };
    }
    return { busy: false, hasDocument: true };
  });
  app.context.window.addEventListener = () => {};
  app.context.window.dawsuUi = { commands: commandList };
  const { menuCalls } = initMenus(app.context, { ui: app.context.window.dawsuUi });
  const done = menuCalls.createHsDiffFromCurrent();
  await Promise.race([done, new Promise((_, reject) => setTimeout(() => reject(new Error("createHsDiffFromCurrent timed out (run queue deadlocked)")), 500))]);
  assert.deepEqual(opened, ["F:/Songs/x/A - B (C) [Hitsounds].osu"]);
});

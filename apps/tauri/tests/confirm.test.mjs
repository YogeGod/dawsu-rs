import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";
import { bootEditor } from "./editor-runtime.mjs";
import { hasOpenModal, onlyModalOpen } from "../ui/src/lib/registry.js";

// The dialog box sits at (100,100)-(500,400); its padding is inside that box.
const DIALOG_BOX = { left: 100, top: 100, right: 500, bottom: 400, width: 400, height: 300, x: 100, y: 100 };

function element(tag = "div") {
  const el = {
    tagName: tag.toUpperCase(), events: new Map(), dataset: {}, children: [], open: false,
    className: "", textContent: "",
    classList: {
      add(name) { if (!el.classList.contains(name)) el.className = (el.className + " " + name).trim(); },
      remove(name) { el.className = el.className.split(/\s+/).filter(one => one !== name).join(" "); },
      contains(name) { return el.className.split(/\s+/).includes(name); },
      toggle() {},
    },
    append() {}, prepend() {}, setAttribute(name, value) { el[name] = value; },
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

async function launch() {
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
      core: { invoke: async () => ({ busy: false, hasDocument: false }) },
      event: { listen() {} },
    } },
    ResizeObserver: class { observe() {} },
    setInterval() {},
    requestAnimationFrame() {},
  });
  await bootEditor(context);
  const dialog = document.getElementById("dialog");
  return {
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

import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";

const app = await readFile(new URL("../dist/app.js", import.meta.url), "utf8");

test("Audio changes the named output only when transport is stopped", async () => {
  const elements = new Map();
  const calls = [];
  function element(tag) {
    return {
      tag, listeners: new Map(), children: [], dataset: {}, style: {},
      addEventListener(name, listener) { this.listeners.set(name, listener); },
      setAttribute(name, value) { this[name] = value; },
      append(...children) { this.children.push(...children); },
      prepend(...children) { this.children.unshift(...children); },
      replaceChildren(...children) { this.children = children; },
      showModal() { this.open = true; },
      close() { this.open = false; },
    };
  }
  const get = id => {
    if (!elements.has(id)) elements.set(id, element("div"));
    return elements.get(id);
  };
  const context = createContext({
    document: { getElementById: get, createElement: element, querySelectorAll: () => [], addEventListener() {} },
    window: { __TAURI__: {
      core: { invoke: async (name, args) => {
        calls.push([name, JSON.parse(JSON.stringify(args ?? null))]);
        if (name === "audio_outputs") return {current: "Speakers", devices: ["Speakers", "Headphones"], bufferMs: 30};
        return {busy: false, hasDocument: false, position: 0, playing: false};
      } },
      event: { listen() {} },
    } },
    ResizeObserver: class { observe() {} },
    requestAnimationFrame() {},
    setInterval() {},
    console,
  });
  await runInContext(app, context);
  calls.length = 0;
  const open = get("audio").listeners.get("click");
  const control = label => get("dialog-body").children
    .find(node => node.children?.[0] === label)?.children[1];
  const choice = () => control("Output device");

  runInContext("view.playing = true", context);
  open();
  await runInContext("pending", context);
  assert.deepEqual(calls.at(-1), ["audio_outputs", null]);
  assert.ok(choice(), "Audio should show an output selector");
  assert.equal(choice().disabled, true);
  assert.equal(control("Requested buffer (ms)").disabled, true);
  assert.equal(choice().value, "Speakers");
  assert.deepEqual(choice().children.map(option => option.textContent), ["Speakers", "Headphones"]);
  choice().value = "Headphones";
  choice().listeners.get("change")();
  await runInContext("pending", context);
  assert.equal(calls.filter(([, args]) => args?.action?.kind === "output_device").length, 0);

  runInContext("closeModal(); view.playing = false", context);
  open();
  await runInContext("pending", context);
  assert.equal(choice().disabled, false);
  choice().value = "Headphones";
  choice().listeners.get("change")();
  await runInContext("pending", context);
  assert.deepEqual(calls.at(-1), ["editor_view", {scene: false, action: {kind: "output_device", name: "Headphones"}}]);

  const buffer = control("Requested buffer (ms)");
  assert.equal(buffer.disabled, false);
  assert.equal(buffer.value, 30);
  buffer.value = "40";
  buffer.listeners.get("change")();
  await runInContext("pending", context);
  assert.deepEqual(calls.at(-1), ["editor_view", {scene: false, action: {kind: "output_buffer", ms: 40}}]);
  buffer.value = "";
  buffer.listeners.get("change")();
  await runInContext("pending", context);
  assert.deepEqual(calls.at(-1), ["editor_view", {scene: false, action: {kind: "output_buffer", ms: null}}]);
});
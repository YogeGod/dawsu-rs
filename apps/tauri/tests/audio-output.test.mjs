import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";

const app = await readFile(new URL("../dist/app.js", import.meta.url), "utf8");

test("Audio configures backend, qualified device and buffer together when transport is stopped", async () => {
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
        if (name === "audio_outputs") {
          const backend = args?.backend ?? "system";
          return {
            backend, available: [{id: "system", label: "WASAPI"}, {id: "mme", label: "MME"}, {id: "asio", label: "ASIO"}],
            current: "Speakers", currentBackend: "WASAPI", device: backend === "system" ? "wasapi:speakers" : null,
            devices: [{id: backend === "system" ? "wasapi:speakers" : backend + ":speakers", name: "Speakers"}],
            bufferMs: 30, effectiveBufferMs: 32,
          };
        }
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
  assert.deepEqual(calls.at(-1), ["audio_outputs", {backend: null}]);
  assert.equal(control("Output backend").disabled, true);
  assert.equal(choice().disabled, true);
  assert.equal(control("Requested buffer (ms)").disabled, true);
  assert.equal(choice().value, "wasapi:speakers");

  const apply = () => get("dialog-body").children.find(node => node.textContent === "Apply output");
  assert.equal(apply().disabled, true);
  apply().listeners.get("click")();
  await runInContext("pending", context);
  assert.equal(calls.filter(([, args]) => args?.action?.kind === "output_settings").length, 0);

  runInContext("closeModal(); view.playing = false", context);
  open();
  await runInContext("pending", context);
  const backend = control("Output backend");
  assert.deepEqual(backend.children.map(option => option.textContent), ["WASAPI", "MME", "ASIO"]);
  backend.value = "mme";
  backend.listeners.get("change")();
  await runInContext("pending", context);
  assert.deepEqual(calls.at(-1), ["audio_outputs", {backend: "mme"}]);
  assert.deepEqual(choice().children.map(option => option.textContent), ["Device default", "Speakers"]);
  choice().value = "mme:speakers";
  const buffer = control("Requested buffer (ms)");
  assert.equal(buffer.value, 30);
  buffer.value = "40";
  apply().listeners.get("click")();
  await runInContext("pending", context);
  assert.ok(calls.some(([name, args]) => name === "editor_view" && args.action?.kind === "output_settings"
    && args.action.backend === "mme" && args.action.device_id === "mme:speakers" && args.action.ms === 40));

  backend.value = "asio";
  backend.listeners.get("change")();
  await runInContext("pending", context);
  assert.equal(choice().children[0].textContent, "Choose an ASIO driver");
  assert.equal(apply().disabled, true);
  choice().value = "asio:speakers";
  choice().listeners.get("change")();
  assert.equal(apply().disabled, false);
});

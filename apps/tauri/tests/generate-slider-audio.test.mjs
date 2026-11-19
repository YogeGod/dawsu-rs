import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";

const app = await readFile(new URL("../dist/app.js", import.meta.url), "utf8");

test("Generate reviews hits only by default and includes slider audio only on request", async () => {
  const calls = [];
  const elements = new Map();
  const makeElement = () => ({
    events: new Map(), children: [],
    addEventListener(name, callback) { this.events.set(name, callback); },
    append(...children) { this.children.push(...children); },
    prepend(child) { this.children.unshift(child); },
  });
  const get = id => {
    if (!elements.has(id)) elements.set(id, makeElement());
    return elements.get(id);
  };
  const context = createContext({
    document: {
      getElementById: get,
      createElement: makeElement,
      querySelectorAll: () => [],
      addEventListener() {},
    },
    window: { __TAURI__: {
      core: { invoke: async (name, args) => {
        calls.push(args.action);
        return { busy: false, hasDocument: false };
      } },
      event: { listen() {} },
    } },
    ResizeObserver: class { observe() {} },
    setInterval() {},
    requestAnimationFrame() {},
  });
  await runInContext(app, context);
  calls.length = 0;
  for (const include of [false, true]) {
    runInContext('scene = {path: "Sliders.osu"}; view = {}; ' +
      'modal = (_kind, title, content, buttons) => { globalThis.generate = { title, content, buttons }; }; ' +
      'closeModal = () => {};', context);
    get("generate").events.get("click")();
    assert.equal(context.generate.title, "Generate hitsound difficulty");
    assert.equal(context.generate.content[0].textContent, "Generate from Sliders.osu");
    const [, checkbox] = context.generate.content.find(node => node.children[0] === "Include slider slide, whistle and tick audio").children;
    assert.equal(checkbox.checked, false);
    checkbox.checked = include;
    context.generate.buttons.find(([label]) => label === "Review output")[1]();
    await runInContext("pending", context);
    assert.deepEqual(JSON.parse(JSON.stringify(calls.at(-1))), { kind: "generate", include_slider_audio: include });
  }
});

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";
import { copyOptions, defaultChoices } from "../ui/src/lib/export.js";

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
  runInContext(read("dist/app.js"), context);
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
  get("copy").events.get("click")();
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
  assert.doesNotMatch(read("dist/style.css"), /Inter-Variable/);
});

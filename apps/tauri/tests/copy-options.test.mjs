import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";

const app = readFileSync(new URL("../dist/app.js", import.meta.url), "utf8");

test("Copy picks target difficulties and sends the chosen output policy for review", async () => {
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
    document: {
      getElementById: get,
      createElement: makeElement,
      querySelectorAll: () => [],
      addEventListener() {},
    },
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
  await runInContext(app, context);
  calls.length = 0;
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
  const controls = Object.fromEntries(Array.from(descendants(context.copy.content))
    .filter(node => typeof node.children[0] === "string" && node.children[1])
    .map(node => [node.children[0], node.children[1]]));
  assert.equal(controls["Target.osu"].type, "checkbox");
  assert.equal(controls["Timing tolerance (ms)"].value, 5);
  assert.ok(controls["Unmatched target hits"], "choose whether unmatched target hits stay as they are");
  assert.equal(controls["Copy volumes"].checked, true);
  assert.equal(controls["Copy storyboard samples"].checked, false);
  controls["Target.osu"].checked = true;
  controls["Unmatched target hits"].value = "overwrite_only_defined";
  controls["Timing tolerance (ms)"].value = 3;
  controls["Copy volumes"].checked = false;
  controls["Copy volumes"].events.get("change")();
  assert.equal(controls["Keep target hits at 5%"].disabled, true);
  controls["Copy storyboard samples"].checked = true;
  controls["Copy storyboard samples"].events.get("change")();
  assert.equal(controls["Skip samples at any hit"].disabled, false);
  controls["Skip samples at any hit"].checked = true;
  controls["Mute unmatched slider and spinner ends"].checked = true;
  controls["Mute unmatched slider and spinner ends"].events.get("change")();
  const [, submit] = context.copy.buttons.find(([label]) => label === "Review output");
  controls["Beat divisors (highest priority first)"].value = "4, 0";
  submit();
  await runInContext("pending", context);
  assert.equal(calls.length, 0, "invalid divisors must leave the dialog open");
  controls["Beat divisors (highest priority first)"].value = "4, 1, 3";
  controls["Muted beat divisors"].value = "4, 3";
  controls["Minimum slider length (beats)"].value = 0.75;
  controls["Muted sample index"].value = 7;
  controls["Muted sample set"].value = "drum";
  submit();
  await runInContext("pending", context);
  assert.equal(calls[0].name, "editor_view");
  assert.deepEqual(JSON.parse(JSON.stringify(calls[0].args.action)), {
    kind: "copy",
    targets: ["Target.osu"],
    options: {
      mode: "overwrite_only_defined",
      leniency: 3,
      copy_hitsounds: true,
      copy_body_hitsounds: true,
      copy_sample_sets: true,
      copy_volumes: false,
      always_preserve_5_volume: true,
      copy_storyboarded_samples: true,
      ignore_hitsound_satisfied_samples: true,
      ignore_whenever_hitsound: true,
      mute_slider_ends: true,
      beat_divisors: [4, 1, 3],
      muted_divisors: [4, 3],
      min_length: 0.75,
      muted_index: 7,
      muted_sample_set: "drum",
    },
  });
});

import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";

const read = file => readFile(new URL("../dist/" + file, import.meta.url), "utf8");
const [html, css, app, startScreenSvelte] = await Promise.all([
  read("index.html"), read("style.css"), read("app.js"),
  readFile(new URL("../ui/src/parts/StartScreen.svelte", import.meta.url), "utf8"),
]);
const homeCss = css.slice(css.indexOf("/* Start and empty screen (design f4)"));

async function frontend() {
  const controls = new Map();
  const calls = [];
  function element() {
    return {
      events: new Map(), children: [], dataset: {}, style: { setProperty() {} },
      addEventListener(name, listener) { this.events.set(name, listener); },
      append(...children) { this.children.push(...children); }, prepend() {},
      replaceChildren(...children) { this.children = children; },
      setAttribute(name, value) { this[name] = value; },
      classList: { toggle() {}, add() {}, remove() {} },
    };
  }
  const get = id => {
    if (!controls.has(id)) controls.set(id, element());
    return controls.get(id);
  };
  const context = createContext({
    document: { getElementById: get, createElement: element, querySelectorAll: () => [], addEventListener() {} },
    window: { __TAURI__: {
      core: { invoke: async (name, args) => {
        calls.push({ name, args: JSON.parse(JSON.stringify(args ?? null)) });
        return { busy: false, hasDocument: false };
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
  const evaluate = code => runInContext(code, context);
  return { calls, evaluate, get, settle: () => evaluate("pending") };
}

const mapPath = "C:\\Songs\\1 yuikonnu - Chikyuu\\yuikonnu - Chikyuu Saigo no Kokuhaku wo (RealCuT) [Kokuhaku in 2019].osu";

test("the empty screen mounts StartScreen in #home with Open map, Import .osz, Folders, drop zone and recent maps", () => {
  assert.match(html, /<section id="home" data-ui-mount="start-screen"><\/section>/);
  const actions = [...startScreenSvelte.matchAll(/<button id="([\w-]+)"[^>]*class="home-action"/g)].map(match => match[1]);
  assert.deepEqual(actions, ["home-open", "home-import", "home-folders"]);
  assert.doesNotMatch(startScreenSvelte, /Open from osu!/);
  assert.match(startScreenSvelte, /class="home-drop"[\s\S]*?\.osu[\s\S]*?\.osz/);
  assert.match(startScreenSvelte, /id="home-recent"[\s\S]*?id="recent"/);
  for (const id of ["home-resume", "recovery", "recovery-entries"]) assert.match(startScreenSvelte, new RegExp('id="' + id + '"'), id);
  for (const dead of ["function renderRecent", "function renderRecovery"]) assert.ok(!app.includes(dead), dead);
  assert.match(homeCss, /\.home-drop\s*\{[^}]*dashed/);
  assert.doesNotMatch(homeCss, /:has\(|color-mix\(|aspect-ratio|url\(/);
});

test("startScreenAction opens recent maps, imports .osz archives and opens Folders", async () => {
  const { calls, evaluate, settle } = await frontend();
  evaluate("var folders = 0; openFolders = () => { folders++; };");
  evaluate(`startScreenAction("openRecent", ${JSON.stringify(mapPath)})`);
  await settle();
  assert.deepEqual(calls.at(-1), { name: "editor_view", args: { scene: true, action: { kind: "open", path: mapPath } } });
  evaluate('startScreenAction("importOsz")');
  await settle();
  assert.deepEqual(calls.at(-1), { name: "pick_map", args: { archiveOnly: true } });
  evaluate('startScreenAction("folders")');
  assert.equal(evaluate("folders"), 1);
});

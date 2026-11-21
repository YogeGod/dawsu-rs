import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";

const read = file => readFile(new URL("../dist/" + file, import.meta.url), "utf8");
const [html, css, app] = await Promise.all(["index.html", "style.css", "app.js"].map(read));
const home = html.match(/<section id="home">.*?<section id="workspace"/s)?.[0] ?? "";
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

test("the empty screen offers Open map, Import .osz and Folders, a drop zone and recent maps", () => {
  const actions = [...home.matchAll(/<button id="([\w-]+)"[^>]*class="home-action"/g)].map(match => match[1]);
  assert.deepEqual(actions, ["home-open", "home-import", "home-folders"]);
  assert.doesNotMatch(home, /Open from osu!/);
  assert.match(home, /class="home-drop"[^>]*>[\s\S]*?\.osu[\s\S]*?\.osz/);
  assert.match(home, /id="home-recent"[^>]*hidden[\s\S]*?id="recent"/);
  for (const id of ["home-resume", "recovery", "recovery-entries"]) assert.match(home, new RegExp('id="' + id + '"'), id);
  assert.match(homeCss, /\.home-drop\s*\{[^}]*dashed/);
  assert.doesNotMatch(homeCss, /:has\(|color-mix\(|aspect-ratio|url\(/);
});

test("recent maps show the map name and difficulty from the file name and open on click", async () => {
  const { calls, evaluate, get, settle } = await frontend();
  evaluate(`show({ busy: false, hasDocument: false, recent: [${JSON.stringify(mapPath)}, "C:/maps/recent.osu", "C:/maps/Some Pack.OSZ"] })`);
  assert.equal(get("home-recent").hidden, false);
  const [first, second] = get("recent").children;
  assert.equal(first.title, mapPath);
  assert.deepEqual(first.children.map(child => child.textContent), ["yuikonnu - Chikyuu Saigo no Kokuhaku wo", "[Kokuhaku in 2019]"]);
  assert.deepEqual(second.children.map(child => child.textContent), ["recent", ""]);
  assert.deepEqual(get("recent").children[2].children.map(child => child.textContent), ["Some Pack", ""]);
  first.events.get("click")();
  await settle();
  assert.deepEqual(calls.at(-1), { name: "editor_view", args: { scene: true, action: { kind: "open", path: mapPath } } });

  evaluate("show({ busy: false, hasDocument: false, recent: [] })");
  assert.equal(get("home-recent").hidden, true);
});

test("Import .osz picks an archive and Folders opens the Folders dialog", async () => {
  const { calls, evaluate, get, settle } = await frontend();
  evaluate("var folders = 0; openFolders = () => { folders++; };");
  get("home-import").events.get("click")();
  await settle();
  assert.deepEqual(calls.at(-1), { name: "pick_map", args: { archiveOnly: true } });
  get("home-folders").events.get("click")();
  assert.equal(evaluate("folders"), 1);

  evaluate("show({ busy: true, hasDocument: false, recent: [] })");
  assert.equal(get("home-import").disabled, true);
  assert.equal(get("home-folders").disabled, true);
});

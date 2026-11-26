import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";
import { bootEditor } from "./editor-runtime.mjs";

const read = file => readFile(new URL("../dist/" + file, import.meta.url), "utf8");
const [html, css, startScreenSvelte] = await Promise.all([
  read("index.html"), readFile(new URL("../ui/src/legacy-chrome.css", import.meta.url), "utf8"),
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
  const store = new Map();
  const toasts = [];
  const context = createContext({
    document: { getElementById: get, createElement: element, querySelectorAll: () => [], addEventListener() {} },
    localStorage: {
      getItem: key => store.get(key) ?? null,
      setItem: (key, value) => store.set(key, String(value)),
    },
    window: {
      showToast: toast => toasts.push(toast),
      __TAURI__: {
        core: { invoke: async (name, args) => {
          calls.push({ name, args: JSON.parse(JSON.stringify(args ?? null)) });
          return { busy: false, hasDocument: false };
        } },
        event: { listen() {} },
      },
    },
    ResizeObserver: class { observe() {} },
    requestAnimationFrame() {},
    setInterval() {},
    console,
  });
  await bootEditor(context);
  calls.length = 0;
  const evaluate = code => runInContext(code, context);
  return { calls, evaluate, get, store, toasts, settle: () => evaluate("pending") };
}

const mapPath = "C:\\Songs\\1 yuikonnu - Chikyuu\\yuikonnu - Chikyuu Saigo no Kokuhaku wo (RealCuT) [Kokuhaku in 2019].osu";

test("the empty screen mounts StartScreen in #home with Open map, Try practice mapset, Import .osz, Folders, drop zone and recent maps", () => {
  assert.match(html, /<section id="home" data-ui-mount="start-screen"><\/section>/);
  const actions = [...startScreenSvelte.matchAll(/<button id="([\w-]+)"[^>]*class="home-action"/g)].map(match => match[1]);
  assert.deepEqual(actions, ["home-open", "home-create-hs", "home-open-osu", "home-import", "home-folders", "home-practice"]);
  assert.match(startScreenSvelte, /Create Hitsound diff…/);
  assert.match(startScreenSvelte, /Open from osu!/);
  assert.match(startScreenSvelte, /Try practice mapset/);
  assert.match(startScreenSvelte, /class="home-drop"[\s\S]*?\.osu[\s\S]*?\.osz/);
  assert.match(startScreenSvelte, /id="home-recent"[\s\S]*?id="recent"/);
  for (const id of ["home-resume", "recovery", "recovery-entries"]) assert.match(startScreenSvelte, new RegExp('id="' + id + '"'), id);
  assert.match(homeCss, /\.home-drop\s*\{[^}]*dashed/);
  assert.doesNotMatch(homeCss, /:has\(|color-mix\(|aspect-ratio|url\(/);
});

test("startScreenAction opens recent maps, imports .osz archives, opens Folders/Audio, and launches practice mapset", async () => {
  const { calls, evaluate, store, toasts, settle } = await frontend();
  evaluate("var folders = 0; openFolders = () => { folders++; };");
  evaluate(`startScreenAction("openRecent", ${JSON.stringify(mapPath)})`);
  await settle();
  assert.deepEqual(calls.at(-1), { name: "editor_view", args: { scene: true, action: { kind: "open", path: mapPath } } });
  evaluate('startScreenAction("importOsz")');
  await settle();
  assert.deepEqual(calls.at(-1), { name: "pick_map", args: { archiveOnly: true } });
  evaluate('startScreenAction("folders")');
  assert.equal(evaluate("folders"), 1);
  evaluate('var prefsTab = ""; openPreferences = tab => { prefsTab = tab; };');
  evaluate('startScreenAction("shortcuts")');
  assert.equal(evaluate("prefsTab"), "shortcuts");
  evaluate('startScreenAction("audio")');
  assert.equal(evaluate("prefsTab"), "audio");
  evaluate('startScreenAction("tryPractice")');
  await settle();
  assert.deepEqual(calls.at(-1), { name: "editor_view", args: { scene: true, action: { kind: "open_practice_mapset" } } });

  // Opening a practice mapset announces once per session folder and records dawsu.practiceTried before ui.update.
  evaluate('renderScene = () => {}; renderSelection = () => {};');
  evaluate('show({ hasDocument: true, showHome: false, isPracticeMapset: true, scene: { path: "C:/Temp/dawsu-practice/session-1/[Hard].osu", lanes: [], difficulties: [], moments: [], sections: [] } })');
  assert.equal(store.get("dawsu.practiceTried"), "1");
  assert.equal(toasts.length, 1);
  assert.equal(toasts[0].title, "Practice mapset opened");
  // Switching sibling difficulty or parking & resuming keeps toast count at 1.
  evaluate('show({ hasDocument: true, showHome: false, isPracticeMapset: true, scene: { path: "C:/Temp/dawsu-practice/session-1/[Normal].osu", lanes: [], difficulties: [], moments: [], sections: [] } })');
  evaluate('show({ hasDocument: true, showHome: true, isPracticeMapset: true, scene: null })');
  evaluate('show({ hasDocument: true, showHome: false, isPracticeMapset: true, scene: { path: "C:/Temp/dawsu-practice/session-1/[Normal].osu", lanes: [], difficulties: [], moments: [], sections: [] } })');
  assert.equal(toasts.length, 1);
  // Launching a fresh practice session folder announces once more.
  evaluate('show({ hasDocument: true, showHome: false, isPracticeMapset: true, scene: { path: "C:/Temp/dawsu-practice/session-2/[Hard].osu", lanes: [], difficulties: [], moments: [], sections: [] } })');
  assert.equal(toasts.length, 2);
});

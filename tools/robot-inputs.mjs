// Every input a mapper can give dawsu!, read from the code, and which of them the robot mapper uses.
//
// node tools/robot-inputs.mjs [--md <file>] [--json <file>] [--run <every-input results.json>]...
//   [--robot <script.mjs>]...
//
//   --md     write the inventory and the coverage table as Markdown
//   --json   write them as data
//   --run    a results file from tools/every-input.mjs (or everyday-check --json): count only what
//            that run really did (its steps' `covers`, and the controls its crawl pressed), not what
//            the scripts say they try
//   --robot  the robot scripts to read `covers: [...]` lists from (default: tools/everyday-check.mjs
//            and tools/every-input.mjs)
//
// The inventory has eight kinds of input:
//   key      every command in the command list (apps/tauri/ui/src/lib/commands.js, which a Rust test
//            keeps equal to the Rust catalogue) in each key preset, DAW-like and osu!
//   menu     every row of the five menus, the tool picker and the lane right-click menu
//   palette  Ctrl+K: opening, searching, choosing with the keys or the mouse, and running every entry
//   mouse    every mouse action the Rust catalogue lists for help (click, drag, wheel, right-click...)
//   control  every button, field, list and switch drawn by the Svelte parts and index.html
//   dialog   every dialog the editor's page opens (modal("kind", "Title", ...))
//   window   the editor's other windows (export, drum, node graph, map preview)
//   feature  the features added on 2026-11-24 that need more than one input to reach
//
// A robot step names what it covers in `covers: [...]`, with these ids. "kind:*" or "kind:part:*"
// claims every entry of that kind or part (the crawl and the key sweep go through them all at run
// time); --run then shows what the run reached.
import { existsSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { basename, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repo = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const args = { run: [], robot: [] };
for (let i = 2; i < process.argv.length; i++) {
  const name = process.argv[i].replace(/^--/, "");
  const value = process.argv[i + 1];
  if (Array.isArray(args[name])) args[name].push(value);
  else args[name] = value;
  i++;
}
if (!args.robot.length) args.robot = ["tools/everyday-check.mjs", "tools/every-input.mjs"];

const text = path => readFileSync(join(repo, path), "utf8");

// The command list, without importing it (commands.js has no imports, but keep this a plain read).
const commandsJs = text("apps/tauri/ui/src/lib/commands.js");
const itemsBlock = commandsJs.slice(commandsJs.indexOf("export const items"), commandsJs.indexOf("export const menus"));
const items = [...itemsBlock.matchAll(/\{\s*id:\s*"([^"]+)",\s*label:\s*"([^"]+)"(.*?)\},?\s*$/gm)].map(([, id, label, rest]) => ({
  id, label, command: /command:\s*"([^"]+)"/.exec(rest)?.[1] ?? null,
}));
const menusBlock = commandsJs.slice(commandsJs.indexOf("export const menus"), commandsJs.indexOf("export const keyOnly"));
const menus = Object.fromEntries([...menusBlock.matchAll(/(\w+):\s*\[([^\]]*)\]/g)].map(([, name, list]) =>
  [name, [...list.matchAll(/"([^"]+)"/g)].map(m => m[1]).filter(id => id !== "-")]));
const keyOnlyBlock = commandsJs.slice(commandsJs.indexOf("export const keyOnly"));
const keyOnly = [...keyOnlyBlock.matchAll(/"([^"]+)"/g)].map(m => m[1]);
const catalogue = [...new Set([...items.map(item => item.command).filter(Boolean), ...keyOnly])].sort();
const label = Object.fromEntries(items.map(item => [item.id, item.label]));

const inventory = [];
const add = (id, kind, what, where) => inventory.push({ id, kind, what, where });

for (const preset of ["DAW", "osu!"]) {
  for (const command of catalogue) {
    const row = items.find(item => item.command === command);
    add(`key:${command}@${preset}`, "key", `${row ? row.label : command}, ${preset === "DAW" ? "DAW-like" : "osu!"} keys`, "Rust catalogue");
  }
}
// Keys the page itself handles before the catalogue (apps/tauri/ui/src/editor/boot.js and the parts).
for (const [id, what] of [
  ["key:page:4-tools-menu", "4 opens the More tools menu"],
  ["key:page:N-next-issue", "N steps to the next coverage issue"],
  ["key:page:Shift+N-previous-issue", "Shift+N steps to the previous coverage issue"],
  ["key:page:Escape-cancels-drag", "Escape cancels a drag in progress"],
  ["key:page:Escape-resumes-from-home", "Escape on the start screen goes back to the open map"],
  ["key:page:Space-not-a-button", "Space plays even when a toolbar button has focus"],
  ["key:page:Tab-routes", "Tab and Shift+Tab move between the editor's areas"],
  ["key:page:Alt-held-draws", "Holding Alt shows Draw while Select is on"],
  ["key:page:Ctrl+K", "Ctrl+K opens and closes the command palette"],
  ["key:page:Ctrl+=/-/0-scale", "Ctrl+= / Ctrl+- / Ctrl+0 change the interface scale"],
  ["key:page:Menu-or-Shift+F10-lane", "Menu key or Shift+F10 opens a lane's menu"],
]) add(id, "key", what, "page");

for (const [menu, ids] of Object.entries(menus)) {
  for (const id of ids) add(`menu:${menu}.${id}`, "menu", `${menu} › ${label[id] ?? id}`, "commands.js");
}

for (const [id, what] of [
  ["palette:open-with-ctrl-k", "Ctrl+K opens it"],
  ["palette:type-to-search", "typing filters the list"],
  ["palette:arrow-keys", "Up and Down move the choice"],
  ["palette:enter-runs", "Enter runs the chosen command"],
  ["palette:click-runs", "a click runs a command"],
  ["palette:escape-closes", "Escape closes it"],
  ["palette:backdrop-closes", "a click outside closes it"],
  ["palette:ctrl-k-closes", "Ctrl+K again closes it"],
]) add(id, "palette", what, "CommandPalette.svelte");
// The palette lists the rows of the five menus and the tool picker, but not Edit layout… or choice rows
// (apps/tauri/ui/src/lib/commandPalette.js), then the key-only commands no row stands for.
const choiceRows = new Set([...itemsBlock.matchAll(/id:\s*"([^"]+)"[^\n]*choices:/g)].map(m => m[1]));
for (const id of [...new Set(Object.entries(menus).filter(([name]) => name !== "lane").flatMap(([, ids]) => ids))]
  .filter(id => id !== "editLayout" && !choiceRows.has(id))) {
  add(`palette:run:${id}`, "palette", `run "${label[id] ?? id}" from Ctrl+K`, "commandPalette.js");
}
const inMenus = new Set(items.map(item => item.command).filter(Boolean));
for (const id of keyOnly.filter(id => !inMenus.has(id))) add(`palette:run:${id}`, "palette", `run the key-only command ${id} from Ctrl+K`, "commandPalette.js");

// Mouse actions: what the editor tells the mapper in Shortcuts and mouse actions.
for (const file of ["crates/dawsu-application/src/commands.rs", "crates/dawsu-editor/src/tools.rs"]) {
  const source = text(file);
  for (const [, gesture] of source.matchAll(/gesture:\s*"([^"]+)"/g)) add(`mouse:${gesture}`, "mouse", gesture, basename(file));
  for (const [, gesture] of source.matchAll(/m!\(\s*"([^"]+)"/g)) add(`mouse:${gesture}`, "mouse", gesture, basename(file));
}
// Mouse input the help does not list.
for (const [id, what] of [
  ["mouse:Hover a hit or lane (hint bar)", "the status bar's hint follows the pointer"],
  ["mouse:Drag a panel edge", "resize the Browser, Inspector or bottom panel"],
  ["mouse:Right-click the stem lane", "clear the drum stem"],
  ["mouse:Drop a file on the window", "open a map, or replace a sample, by dropping a file"],
  ["mouse:Wheel over the snap slider", "change the snap"],
  ["mouse:Drag the playhead", "scrub"],
  ["mouse:Draw tool paint stroke", "one stroke places a hit on every snap step crossed"],
]) add(id, "mouse", what, "page");

// Controls drawn by the Svelte parts and the main page.
const windowOf = {
  ExportWindow: "export", DrumDetectWindow: "drum", NodeGraphWindow: "node-graph", PreviewWindow: "preview", PreviewCanvas: "preview",
};
const partsDir = "apps/tauri/ui/src/parts";
const sources = [...readdirSync(join(repo, partsDir)).filter(name => name.endsWith(".svelte")).map(name => join(partsDir, name)),
  "apps/tauri/dist/index.html"];
const clean = value => value.replace(/\{[^}]*\}/g, "{…}").replace(/<[^>]+>/g, " ").replace(/&[a-z]+;/g, " ").replace(/\s+/g, " ").trim();
for (const path of sources) {
  const part = basename(path).replace(/\.(svelte|html)$/, "");
  const source = text(path).replace(/<script[\s\S]*?<\/script>/g, "").replace(/<style[\s\S]*?<\/style>/g, "").replace(/<!--[\s\S]*?-->/g, "");
  const seen = new Map();
  for (const match of source.matchAll(/<(button|input|select|textarea|summary)\b((?:[^>{]|\{[^}]*\})*)>/g)) {
    const [, tag, attrs] = match;
    if (/type="hidden"/.test(attrs)) continue;
    const attr = name => new RegExp(`\\b${name}="([^"]*)"`).exec(attrs)?.[1] ?? (new RegExp(`\\b${name}=\\{([^}]*)\\}`).exec(attrs) ? "{…}" : null);
    let name = attr("aria-label") ?? attr("title") ?? attr("placeholder") ?? null;
    if ((!name || name === "{…}") && (tag === "button" || tag === "summary")) {
      const close = source.indexOf(`</${tag}>`, match.index);
      const inner = clean(source.slice(match.index + match[0].length, close));
      if (inner && inner !== "{…}") name = inner;
    }
    if (!name || name === "{…}") name = attr("id") ? `#${attr("id")}` : name;
    if (!name) {
      const before = source.slice(Math.max(0, match.index - 300), match.index);
      const span = /<span[^>]*>([^<{]+)<\/span>\s*$/.exec(before)?.[1] ?? /<label[^>]*>\s*(?:<span[^>]*>)?([^<{]+)[^]*$/.exec(before)?.[1];
      name = span ? clean(span) : "(unnamed)";
    }
    const type = tag === "input" ? (attr("type") ?? "text") : tag;
    let key = `${name} (${type})`;
    seen.set(key, (seen.get(key) ?? 0) + 1);
    if (seen.get(key) > 1) key = `${key} ${seen.get(key)}`;
    const surface = windowOf[part] ?? "main";
    add(`control:${surface}:${part}:${key}`, "control", `${part}: ${key}`, path.replaceAll("\\", "/"));
  }
}

// Dialogs the main page opens.
for (const path of readdirSync(join(repo, "apps/tauri/ui/src/editor")).map(name => `apps/tauri/ui/src/editor/${name}`)
  .concat(["apps/tauri/dist/menus.js", "apps/tauri/dist/docking.js"])) {
  if (!existsSync(join(repo, path))) continue;
  for (const [, kind, title] of text(path).matchAll(/modal\(\s*"([^"]+)",\s*("[^"]*"|[^,]+)/g)) {
    const id = `dialog:${kind}`;
    if (!inventory.some(entry => entry.id === id)) add(id, "dialog", `${kind}: ${clean(title.replace(/"/g, ""))}`, basename(path));
  }
}
for (const [id, what] of [
  ["dialog:preferences", "Preferences (Svelte)"], ["dialog:midi-import", "Import MIDI (Svelte)"],
  ["dialog:keysounds", "Keysounds & VST3 (Svelte)"], ["dialog:shared-base", "Shared Base layer (Svelte)"],
  ["dialog:stem-separation", "Separate drums (AI) (Svelte)"], ["dialog:export-modal", "Export (Svelte, in the main window)"],
  ["dialog:quick-start", "Quick start tour (Svelte)"], ["dialog:command-palette", "Command palette (Svelte)"],
]) add(id, "dialog", what, "parts");

for (const name of readdirSync(join(repo, "apps/tauri/dist")).filter(name => name.endsWith(".html") && name !== "index.html")) {
  add(`window:${name.replace(/\.html$/, "")}`, "window", `the ${name} window`, "apps/tauri/dist");
}

for (const [id, what] of [
  ["feature:save-check", "Save reads the written .osu back before it replaces the file (#60)"],
  ["feature:sample-checks", "osu! sample health checks and auto-fix for staged mixes (#62)"],
  ["feature:review-list", "per-difficulty coverage review list, N / Shift+N (#68)"],
  ["feature:fallback-sound", "fallback sound for objects with no hit, in Export (#74)"],
  ["feature:live-reload", "the open .osu changes on disk and the editor reloads or asks (#72)"],
  ["feature:ogg-export", "Ogg Vorbis and 32-bit float WAV mix export (#65)"],
  ["feature:greenline-cleanup", "clean redundant green lines on Save and Export (#61)"],
  ["feature:drum-split", "separate the drums with the AI model, or attach a drum stem (#79)"],
  ["feature:map-picker", "the start screen's Songs folder mapset picker (#70)"],
  ["feature:mania-import", "import hitsounds from an osu!mania difficulty (#71)"],
  ["feature:slider-end-mute", "mute slider ends when copying hitsounds (#69)"],
  ["feature:shared-base", "Shared Base layer for several difficulties (#78)"],
  ["feature:create-hitsound-diff", "+ Create Hitsound diff on the start screen (#80)"],
  ["feature:stem-monitor", "T switches the Song and Stem monitor (#73)"],
  ["feature:hint-bar", "the status bar's live hover hint (#64)"],
  ["feature:paint-stroke", "Draw's continuous paint stroke (#67)"],
  ["feature:osu-diff-view", "Export review's changed-lines .osu view (#63)"],
]) add(id, "feature", what, "2026-11-24 merges");

// What the robot scripts claim, and what a run did.
function claims(paths) {
  const found = new Set();
  for (const path of paths) {
    if (!existsSync(join(repo, path))) continue;
    const source = text(path);
    for (const [, list] of source.matchAll(/(?:covers:\s*|\},\s*)\[([^\]]*)\]/g)) {
      for (const [, id] of list.matchAll(/"([^"]+)"/g)) found.add(id);
    }
    // A loop that builds its ids at run time says what it goes through in a comment.
    for (const [, list] of source.matchAll(/\/\/ covers-all:(.*)/g)) for (const id of list.trim().split(/\s+/)) found.add(id);
  }
  return found;
}
function ran(files) {
  const found = new Set();
  const labels = [];
  const details = [];
  for (const file of files) {
    const data = JSON.parse(readFileSync(resolve(file), "utf8"));
    for (const step of data.steps ?? []) {
      if (step.status !== "PASS") continue;
      details.push(step.detail ?? "");
      // A wildcard only says what a step tried; a run counts what it reached.
      for (const id of step.covers ?? []) if (!id.includes("*")) found.add(id);
      for (const id of step.reached ?? []) {
        found.add(id);
        const control = /^control:([^:]+)(?::[^:]+)*?:(.+) \(([^)]+)\)$/.exec(id);
        if (control) labels.push({ surface: control[1], name: control[2].toLowerCase(), type: control[3] });
      }
    }
  }
  // A static control is reached when the crawl pressed a control of its window with the same name
  // ({…} in a name stands for any text) and type.
  for (const entry of inventory.filter(entry => entry.kind === "control")) {
    const [, surface, , rest] = /^control:([^:]+):([^:]+):(.+)$/.exec(entry.id);
    const match = /^(.*) \(([^)]+)\)( \d+)?$/.exec(rest);
    if (!match) continue;
    const pattern = new RegExp("^" + match[1].toLowerCase().split("{…}").map(part => part.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")).join(".*") + "$");
    const type = match[2] === "summary" ? "summary" : match[2];
    if (labels.some(label => label.surface === surface && label.type === type && pattern.test(label.name))) found.add(entry.id);
  }
  // A dialog is reached when a step saw its title open.
  const seen = details.join(" ");
  for (const entry of inventory.filter(entry => entry.kind === "dialog")) {
    const title = entry.what.replace(/^[^:]+: /, "").replace(/ \(Svelte.*\)$/, "").replace(/\{…\}.*$/, "").trim();
    if (title && (seen.includes(`"${title}`) || seen.includes(`${title} opened`))) found.add(entry.id);
  }
  return found;
}
const wildcard = id => new RegExp("^" + id.split("*").map(part => part.replace(/[.+?^${}()|[\]\\]/g, "\\$&")).join(".*") + "$");
const covered = (entry, ids) => ids.has(entry.id) || [...ids].some(id => id.includes("*") && wildcard(id).test(entry.id));

const before = claims(["tools/everyday-check.mjs"]);
const after = claims(args.robot);
const reached = args.run.length ? ran(args.run) : null;
const kinds = ["key", "menu", "palette", "mouse", "control", "dialog", "window", "feature"];
const table = kinds.map(kind => {
  const list = inventory.filter(entry => entry.kind === kind);
  const count = ids => list.filter(entry => covered(entry, ids)).length;
  return { kind, total: list.length, before: count(before), after: count(after), reached: reached ? count(reached) : null };
});

const lines = [
  "# Robot mapper: every input",
  "",
  `Read from the code at ${new Date().toISOString().slice(0, 10)} by tools/robot-inputs.mjs.`,
  "",
  `| Kind | Inputs | Robot before | Robot now${reached ? " | Reached in the run" : ""} |`,
  `|---|---:|---:|---:${reached ? "|---:" : ""}|`,
  ...table.map(row => `| ${row.kind} | ${row.total} | ${row.before} | ${row.after}${reached ? ` | ${row.reached}` : ""} |`),
  `| **all** | ${inventory.length} | ${table.reduce((s, r) => s + r.before, 0)} | ${table.reduce((s, r) => s + r.after, 0)}${reached ? ` | ${table.reduce((s, r) => s + r.reached, 0)}` : ""} |`,
  "",
];
for (const kind of kinds) {
  lines.push(`## ${kind}`, "");
  for (const entry of inventory.filter(entry => entry.kind === kind)) {
    const mark = reached ? (covered(entry, reached) ? "x" : " ") : (covered(entry, after) ? "x" : " ");
    lines.push(`- [${mark}] \`${entry.id}\` — ${entry.what}`);
  }
  lines.push("");
}
if (args.md) writeFileSync(resolve(args.md), lines.join("\n"));
if (args.json) writeFileSync(resolve(args.json), JSON.stringify({ table, inventory }, null, 2) + "\n");
console.log(lines.slice(0, 7 + kinds.length).join("\n"));
if (reached) {
  const missing = inventory.filter(entry => !covered(entry, reached));
  console.log(`\nnot reached in the run: ${missing.length}`);
  for (const entry of missing.slice(0, 400)) console.log(`  ${entry.id}`);
}

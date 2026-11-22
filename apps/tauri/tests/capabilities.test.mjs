import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import { test } from "node:test";

// build.rs lists every command, so a window may call only what its capability file allows.
// The vm fakes answer any invoke, so only this test notices a command main lost.
const tauri = new URL("../", import.meta.url);
const read = path => readFileSync(new URL(path, tauri), "utf8");
const permission = command => "allow-" + command.replaceAll("_", "-");
const capability = name => JSON.parse(read("capabilities/" + name + ".json"));

function sources(dir) {
  return readdirSync(new URL(dir, tauri), { withFileTypes: true }).flatMap(entry => entry.isDirectory()
    ? sources(dir + entry.name + "/")
    : /\.(js|svelte)$/.test(entry.name) ? [{ name: dir + entry.name, text: read(dir + entry.name) }] : []);
}
const invoked = files => new Set(files.flatMap(({ text }) =>
  [...text.matchAll(/invoke\(\s*"([a-z_]+)"/g)].map(match => match[1])));

const exportPage = ["ui/src/lib/ipc.js", "ui/src/lib/export.js", "ui/src/parts/ExportWindow.svelte"];
const exportCommands = ["export_view", "export_preview", "export_run", "close_export_window", "show_in_editor"];
const drumPage = ["ui/src/lib/ipc.js", "ui/src/lib/drum.js", "ui/src/parts/DrumDetectWindow.svelte"];
const previewPage = ["ui/src/lib/ipc.js", "ui/src/parts/PreviewWindow.svelte", "ui/src/parts/PreviewCanvas.svelte"];
const previewCommands = ["map_preview_objects"];
const drumCommands = ["drum_context", "drum_detect_start", "drum_review", "drum_listen",
  "drum_listen_stop", "drum_apply", "drum_pick_stem", "close_drum_window"];
const nodeGraphPage = ["ui/src/lib/ipc.js", "ui/src/lib/node-graph.js", "ui/src/parts/NodeGraphWindow.svelte"];
const nodeGraphCommands = ["node_graph_view", "node_graph_action", "node_graph_export_json",
  "node_graph_pick_open", "node_graph_pick_save", "node_graph_pick_audio", "close_node_graph_window", "show_in_editor"];

test("build.rs lists exactly the commands main.rs registers", () => {
  const handler = read("src/main.rs").match(/generate_handler!\[([^\]]*)\]/)[1].match(/\w+/g);
  const manifest = [...read("build.rs").match(/COMMANDS: &\[&str\] = &\[([^\]]*)\]/)[1].matchAll(/"(\w+)"/g)]
    .map(match => match[1]);
  assert.deepEqual([...manifest].sort(), [...handler].sort());
});

test("main may call every command its scripts and parts call, and opens Export", () => {
  const main = capability("default");
  assert.deepEqual(main.windows, ["main"]);
  const files = [...sources("dist/").filter(file => !file.name.startsWith("dist/ui/")),
    ...sources("ui/src/").filter(file => !exportPage.includes(file.name) && !drumPage.includes(file.name) && !nodeGraphPage.includes(file.name)),
    { name: "ui/src/lib/node-graph.js", text: read("ui/src/lib/node-graph.js") }];
  const needed = [...invoked(files), "open_export_window"];
  assert.ok(needed.length > 20, "the invoke pattern no longer matches editor sources");
  assert.deepEqual(needed.filter(command => !main.permissions.includes(permission(command))), []);
  assert.ok(main.permissions.includes("core:event:default"));
  for (const command of [...exportCommands, ...drumCommands, ...nodeGraphCommands]) assert.ok(!main.permissions.includes(permission(command)), command);
  assert.ok(main.permissions.includes(permission("drum_window_open")));
  assert.ok(main.permissions.includes(permission("open_node_graph_window")));
});

test("the node graph window may call only the node graph commands", () => {
  const ng = capability("node-graph");
  assert.deepEqual(ng.windows, ["node-graph"]);
  assert.deepEqual([...ng.permissions].sort(), ["core:event:default", ...nodeGraphCommands.map(permission)].sort());
  const page = invoked(nodeGraphPage.map(name => ({ name, text: read(name) })));
  assert.deepEqual([...page].sort(), [...nodeGraphCommands].sort(), "the page calls each command it may call");
});

test("the drum detection window may call only the drum commands", () => {
  const drum = capability("drum");
  assert.deepEqual(drum.windows, ["drum"]);
  // It emits "preview-space" to main for Spacebar playback, plus its own drum commands.
  assert.deepEqual(drum.permissions, ["core:event:default", ...drumCommands.map(permission)]);
  const page = invoked(drumPage.map(name => ({ name, text: read(name) })));
  assert.deepEqual([...page].sort(), [...drumCommands].sort(), "the page calls each command it may call");
});

test("the Export window may call only the export commands", () => {
  const exported = capability("export");
  assert.deepEqual(exported.windows, ["export"]);
  // It emits "preview-space" to main for Spacebar playback, plus its own export commands.
  assert.deepEqual(exported.permissions, ["core:event:default", ...exportCommands.map(permission)]);
  const page = invoked(exportPage.map(name => ({ name, text: read(name) })));
  assert.deepEqual([...page].filter(command => !exportCommands.includes(command)), []);
});

test("the map preview window may ask only for the objects, and listen and send its events", () => {
  const preview = capability("preview");
  assert.deepEqual(preview.windows, ["preview"]);
  // It listens for "preview-view" and tells main "preview-ready"; nothing else.
  assert.deepEqual(preview.permissions, ["core:event:default", ...previewCommands.map(permission)]);
  const page = invoked(previewPage.map(name => ({ name, text: read(name) })));
  assert.deepEqual([...page].sort(), [...previewCommands].sort());
  assert.ok(capability("default").permissions.includes(permission("preview_window_open")));
});

test("every invoke names its command literally, so the lists above see every call", () => {
  // ipc.js is the Export page's one wrapper around the core API; its callers pass literals.
  const wrapper = "ui/src/lib/ipc.js";
  const files = [...sources("dist/").filter(file => !file.name.startsWith("dist/ui/")), ...sources("ui/src/")]
    .filter(file => file.name !== wrapper);
  for (const { name, text } of files) {
    const loose = [...text.matchAll(/\binvoke\(\s*(?!["'])/g)].map(match => text.slice(match.index, match.index + 40));
    assert.deepEqual(loose, [], name);
  }
  assert.match(read(wrapper), /core\.invoke\(command, args\)/);
});

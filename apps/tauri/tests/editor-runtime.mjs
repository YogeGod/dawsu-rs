// Shared test harness for apps/tauri/tests: boots the real modular editor runtime
// (apps/tauri/ui/src/editor/boot.js) inside a Node vm context without needing node_modules.
import { readdirSync, readFileSync } from "node:fs";
import { runInContext } from "node:vm";
import { createEditor } from "../ui/src/editor/boot.js";

const editorDir = new URL("../ui/src/editor/", import.meta.url);

export function bootEditor(context, { autoStart = true } = {}) {
  // Expose the vm realm's Date so tests that override Date.now inside runInContext take effect.
  if (!("Date" in context)) context.Date = runInContext("Date", context);
  const ctx = createEditor(context, { autoStart, exposeGlobals: true });
  return autoStart ? ctx.state.pending : ctx;
}

// Reads all modular editor source files concatenated for static assertions in tests.
export function readEditorSource() {
  return readdirSync(editorDir)
    .filter(name => name.endsWith(".js"))
    .sort()
    .map(name => readFileSync(new URL(name, editorDir), "utf8"))
    .join("\n");
}

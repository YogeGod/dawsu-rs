// The build stamp ties the committed bundle in dist/ui/ to its sources. vite.config.js writes it
// after each build; apps/tauri/tests/ui-bundle.test.mjs recomputes it without node_modules.
import { createHash } from "node:crypto";
import { readFileSync, readdirSync } from "node:fs";
import { join, relative, sep } from "node:path";

const tooling = ["package.json", "package-lock.json", "stamp.mjs", "vite.config.js"];
const sha256 = data => createHash("sha256").update(data).digest("hex");

function files(dir) {
  return readdirSync(dir, { withFileTypes: true })
    .flatMap(entry => entry.isDirectory() ? files(join(dir, entry.name)) : [join(dir, entry.name)]);
}

export function buildStamp(uiDir, outDir) {
  const inputs = [...files(join(uiDir, "src")), ...tooling.map(name => join(uiDir, name))]
    .map(path => relative(uiDir, path).split(sep).join("/"))
    .sort();
  const hash = createHash("sha256");
  // Text is hashed with LF line endings, so a Windows checkout with autocrlf gives the same stamp.
  for (const path of inputs) hash.update(path + "\n" + readFileSync(join(uiDir, path), "utf8").replace(/\r\n/g, "\n") + "\0");
  const outputs = Object.fromEntries(["ui.js", "ui.css"].map(name => [name, sha256(readFileSync(join(outDir, name)))]));
  return { inputs: hash.digest("hex"), outputs };
}

import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { compile, compileModule } from "svelte/compiler";

export async function load(url, context, nextLoad) {
  if (url.endsWith(".css")) return { format: "module", source: "", shortCircuit: true };
  if (url.endsWith("?raw")) {
    const text = await readFile(fileURLToPath(url.slice(0, -"?raw".length)), "utf8");
    return { format: "module", source: "export default " + JSON.stringify(text), shortCircuit: true };
  }
  const component = url.endsWith(".svelte");
  if (!component && !url.endsWith(".svelte.js")) return nextLoad(url, context);
  const filename = fileURLToPath(url);
  const source = await readFile(filename, "utf8");
  const { js } = component
    ? compile(source, { filename, generate: "server", css: "external" })
    : compileModule(source, { filename, generate: "server" });
  return { format: "module", source: js.code, shortCircuit: true };
}

import { writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import { defineConfig } from "vite";
import { buildStamp } from "./stamp.mjs";

const uiDir = fileURLToPath(new URL(".", import.meta.url));
const outDir = fileURLToPath(new URL("../dist/ui/", import.meta.url));

// The bundle is committed and compiled into the exe, so cargo builds never need Node.
// A normal build (not library mode) keeps assets out of data: URLs, which the window's CSP blocks.
export default defineConfig({
  base: "./",
  publicDir: false,
  plugins: [
    svelte({ compilerOptions: { css: "external" } }),
    {
      name: "dawsu-build-stamp",
      apply: "build",
      writeBundle() {
        writeFileSync(outDir + "build-stamp.json", JSON.stringify(buildStamp(uiDir, outDir), null, 2) + "\n");
      },
    },
  ],
  build: {
    outDir,
    emptyOutDir: false,
    assetsInlineLimit: 0,
    cssCodeSplit: false,
    modulePreload: false,
    target: ["chrome105", "safari15"],
    rolldownOptions: {
      input: "src/main.js",
      // Vite drops an app entry's exports by default; window.dawsuUi.update is the bridge.
      preserveEntrySignatures: "exports-only",
      output: {
        format: "iife",
        name: "dawsuUi",
        entryFileNames: "ui.js",
        assetFileNames: "ui[extname]",
      },
    },
  },
});

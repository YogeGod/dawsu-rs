#!/usr/bin/env node
// Publishes Mapset Verifier from the local source cache to E:/dawsu-build/mv-oracle/<sha>.
// Usage: node tools/mv-oracle/build.mjs   (env: MV_SOURCE_DIR, MV_ORACLE_OUT)
import { spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import { join } from "node:path";
import { MV_SHORT } from "./oracle.mjs";
import { defaultExe } from "./server.mjs";

const source = process.env.MV_SOURCE_DIR ?? `E:/dawsu-source-cache/Naxesss-MapsetVerifier-${MV_SHORT}`;
const project = join(source, "src", "MapsetVerifier.csproj");
const out = process.env.MV_ORACLE_OUT ?? join(defaultExe(), "..");

if (!existsSync(project)) {
  console.log(`SKIP mv-oracle build: ${project} not found (set MV_SOURCE_DIR)`);
  process.exit(2);
}
const r = spawnSync("dotnet", ["publish", project, "-c", "Release", "-r", "win-x64", "-o", out], { stdio: "inherit", shell: false });
if (r.error) {
  console.log(`SKIP mv-oracle build: dotnet not available (${r.error.message})`);
  process.exit(2);
}
console.log(r.status === 0 ? `built ${out}` : `FAIL mv-oracle build (exit ${r.status})`);
process.exit(r.status ?? 1);

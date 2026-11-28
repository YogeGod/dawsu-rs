#!/usr/bin/env node
// Runs Mapset Verifier on every <cases>/<name>/{before,after} pair and fails on new issues.
// Usage: node tools/mv-oracle/run.mjs [--cases DIR] [--allow-skip] [--timeout-ms N] [--report FILE]
// Exit: 0 all PASS, 1 any FAIL, 2 SKIP (no MV build or no cases) unless --allow-skip.
import { existsSync, readdirSync, readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { join, resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { MV_SHA, evaluateCase } from "./oracle.mjs";
import { startServer, defaultExe } from "./server.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const repo = resolve(here, "..", "..");
const argv = process.argv.slice(2);
const opt = (name, fallback) => {
  const i = argv.indexOf(name);
  return i >= 0 ? argv[i + 1] : fallback;
};
const casesDir = resolve(opt("--cases", join(repo, "target", "mv-oracle", "cases")));
const reportFile = resolve(opt("--report", join(repo, "target", "mv-oracle", "report.json")));
const timeoutMs = Number(opt("--timeout-ms", "120000"));
const allowSkip = argv.includes("--allow-skip");
const exe = defaultExe();

const skip = (why) => {
  console.log(`SKIP mv oracle: ${why}`);
  process.exit(allowSkip ? 0 : 2);
};
if (!existsSync(exe)) skip(`${exe} not found (run node tools/mv-oracle/build.mjs)`);
const cases = existsSync(casesDir)
  ? readdirSync(casesDir, { withFileTypes: true })
      .filter((d) => d.isDirectory() && existsSync(join(casesDir, d.name, "before")) && existsSync(join(casesDir, d.name, "after")))
      .map((d) => d.name)
      .sort()
  : [];
if (!cases.length) skip(`no before/after case folders in ${casesDir}`);

const allowFile = join(here, "expected-new-issues.json");
const allowAll = existsSync(allowFile) ? JSON.parse(readFileSync(allowFile, "utf8")) : [];

let server = await startServer({ exe, timeoutMs });
process.once("SIGINT", () => { server.stop(); process.exit(130); });
const results = [];
try {
  for (const name of cases) {
    const allow = allowAll.filter((a) => a.case === name || a.case === "*").map((a) => a.key);
    const r = await evaluateCase({
      name,
      beforeDir: join(casesDir, name, "before"),
      afterDir: join(casesDir, name, "after"),
      post: server.post,
      allow,
    });
    results.push(r);
    console.log(`${r.result} ${name}${r.reason ? ` - ${r.reason}` : ""}`);
    for (const f of r.failing ?? []) console.log(`    ${f.category} | ${f.check} | ${f.level} | ${f.message}`);
    if (/timed out/.test(r.reason ?? "")) {
      // A hung request may leave MV busy; start a fresh one for the next case.
      server.stop();
      server = await startServer({ exe, timeoutMs });
    }
  }
} finally {
  server.stop();
}

const passed = results.filter((r) => r.result === "PASS").length;
const failed = results.filter((r) => r.result === "FAIL").length;
mkdirSync(dirname(reportFile), { recursive: true });
writeFileSync(reportFile, JSON.stringify({ mvSha: MV_SHA, casesDir, passed, failed, total: results.length, results }, null, 1));
console.log(`mv oracle: ${passed} of ${results.length} cases passed (${failed} failed, ${results.length - passed - failed} skipped) - report ${reportFile}`);
process.exit(failed ? 1 : 0);

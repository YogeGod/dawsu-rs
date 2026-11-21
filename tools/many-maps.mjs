// The robot mapper and the 3.1.4 comparison over many different maps, one after another.
//
// node tools/many-maps.mjs --exe <dawsu-tauri.exe> [--old-app <dawsu-hitsounding-editor checkout>]
//   [--maps <file.osu,folder,...> | --list <text file>] [--json <file>] [--full]
//
//   --maps    .osu files and folders (every .osu directly inside); default: every fixture map in
//             tests/fixtures/save-pack (owner and synthetic) and tests/fixtures/oracle-cases
//   --list    a text file with one .osu file or folder per line (for names with commas)
//   --json    also write the table as data to this file
//   --full    run every everyday-check step on each map (default: --quick, the editing and saving steps)
//
// Each map runs through tools/compare-314.mjs, which copies it first; the fixtures are only read.
// One line per map: the robot's steps, edits and sounds the same as 3.1.4, and the first differences.
// Then the totals. The exit code is 1 when any map failed a robot step or differs from 3.1.4.
import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repo = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const args = {};
for (let i = 2; i < process.argv.length; i++) {
  const value = process.argv[i + 1];
  const flag = value === undefined || value.startsWith("--");
  args[process.argv[i].replace(/^--/, "")] = flag ? true : value;
  if (!flag) i++;
}
if (!args.exe) throw new Error("--exe is required: the built dawsu-tauri.exe");
// --list names a text file with one .osu file or folder per line, for names that hold commas.
const listed = typeof args.list === "string" ? readFileSync(resolve(args.list), "utf8").split(/\r?\n/).map(line => line.trim()).filter(Boolean) : [];
const sources = listed.length ? listed.map(path => resolve(path))
  : typeof args.maps === "string" ? args.maps.split(",").map(path => resolve(path.trim()))
  : ["tests/fixtures/save-pack/owner", "tests/fixtures/save-pack/synthetic", "tests/fixtures/oracle-cases"].map(path => join(repo, path));
const maps = sources.flatMap(path => {
  if (!existsSync(path)) throw new Error(`no map or folder at ${path}`);
  return statSync(path).isDirectory()
    ? readdirSync(path).filter(name => name.toLowerCase().endsWith(".osu")).sort().map(name => join(path, name)) : [path];
});
const out = join(repo, "target", "many-maps");
mkdirSync(out, { recursive: true });

const rows = [];
const started = Date.now();
// After the first map that got through, the twin and the oracle are built: later maps skip the build.
let built = false;
for (const [index, map] of maps.entries()) {
  const name = relative(repo, map).replaceAll("\\", "/");
  console.log(`[${index + 1}/${maps.length}] ${name}`);
  const json = join(out, `${index + 1}.json`);
  rmSync(json, { force: true }); // a file left by an earlier run must not stand in for this map's result
  const run = spawnSync(process.execPath, [join(repo, "tools/compare-314.mjs"), "--exe", args.exe, "--map", map, "--json", json,
    ...(args["old-app"] ? ["--old-app", args["old-app"]] : []), ...(args.full ? [] : ["--quick"]), ...(built ? ["--prebuilt"] : [])],
    { encoding: "utf8", maxBuffer: 64 << 20 });
  const lines = (run.stdout ?? "").trim().split("\n");
  if (!existsSync(json)) {
    const why = (run.stderr ?? "").trim().split("\n").find(line => /Error|error/.test(line)) ?? lines.at(-1) ?? "no output";
    rows.push({ map: name, error: why.slice(0, 300) });
    console.log(`   could not compare: ${why.slice(0, 300)}`);
    continue;
  }
  const result = JSON.parse(readFileSync(json, "utf8"));
  if (result.summary?.sounds?.total) built = true; // the oracle ran, so both tools are built
  const { summary } = result;
  const robot = /(\d+) of (\d+) steps passed/.exec(summary.robot ?? "");
  const differences = [
    ...result.steps.filter(step => !step.same).map(step => `${step.edit}: ${step.text}`),
    ...result.moments.filter(moment => !moment.same).map(moment => `${moment.time} ms: ${moment.notes.join("; ")}`),
  ];
  if (!/saved: true/.test(summary.twin ?? "")) differences.unshift(summary.twin ?? "3.1.4 did not save");
  const row = {
    map: name,
    robot: robot ? { passed: Number(robot[1]), total: Number(robot[2]) } : null,
    robotFailed: summary.robotFailed ?? [],
    twin: summary.twin,
    edits: summary.steps,
    sounds: summary.sounds,
    differences,
  };
  rows.push(row);
  console.log(`   robot ${robot ? `${robot[1]}/${robot[2]}` : "?"} · edits ${summary.steps.same}/${summary.steps.total} same · sounds ${summary.sounds.same}/${summary.sounds.total} same · ${summary.twin}`);
  for (const line of row.robotFailed) console.log(`   robot FAIL ${line}`);
  for (const line of differences.slice(0, 3)) console.log(`   ${line}`);
  if (differences.length > 3) console.log(`   … and ${differences.length - 3} more`);
}

const compared = rows.filter(row => !row.error);
const sum = (pick) => compared.reduce((total, row) => total + pick(row), 0);
const sounds = { same: sum(row => row.sounds.same), total: sum(row => row.sounds.total) };
const edits = { same: sum(row => row.edits.same), total: sum(row => row.edits.total) };
const robotClean = compared.filter(row => row.robot && !row.robotFailed.length).length;
const identical = compared.filter(row => !row.differences.length).length;
console.log("");
console.log(`run time: ${Math.round((Date.now() - started) / 60000)} min`);
console.log(`maps: ${maps.length}; compared ${compared.length}; could not compare ${rows.length - compared.length}`);
console.log(`robot mapper: no failed step on ${robotClean} of ${compared.length} maps`);
console.log(`same as 3.1.4 on every edit and sound: ${identical} of ${compared.length} maps`);
console.log(`edits the same: ${edits.same} of ${edits.total}; sound moments the same: ${sounds.same} of ${sounds.total}` +
  (sounds.total ? ` (${Math.round(100 * sounds.same / sounds.total)}%)` : ""));
if (args.json) writeFileSync(resolve(args.json), JSON.stringify({ maps: rows, totals: { sounds, edits, robotClean, identical, compared: compared.length, total: maps.length } }, null, 2) + "\n");
if (rows.some(row => row.error || row.robotFailed?.length || row.differences?.length)) process.exitCode = 1;

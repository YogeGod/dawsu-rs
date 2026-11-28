// Starts and stops the published Mapset Verifier server as a separate process on loopback.
import { spawn, execFileSync } from "node:child_process";
import { mkdirSync, rmSync } from "node:fs";
import { createServer } from "node:net";
import { MV_SHORT } from "./oracle.mjs";

export function defaultExe() {
  return process.env.MV_ORACLE_EXE ?? `E:/dawsu-build/mv-oracle/${MV_SHORT}/MapsetVerifier.exe`;
}

// shortcut: probing a free port then binding it has a small race; fine for one serial local tool.
const freePort = () =>
  new Promise((res, rej) => {
    const s = createServer().once("error", rej).listen(0, "127.0.0.1", () => {
      const { port } = s.address();
      s.close(() => res(port));
    });
  });

/** Starts MV with custom checks off and a private TEMP/TMP; resolves `{ post, stop }`. */
export async function startServer({ exe = defaultExe(), timeoutMs = 120_000, startupMs = 60_000 } = {}) {
  const port = await freePort();
  const tmp = `E:/dawsu-build/tmp/mv-oracle-${process.pid}-${port}`;
  mkdirSync(tmp, { recursive: true });
  const child = spawn(exe, [`--urls=http://127.0.0.1:${port}`], {
    env: { ...process.env, MAPSET_VERIFIER_CUSTOM_CHECKS_ENABLED: "false", TEMP: tmp, TMP: tmp },
    stdio: ["ignore", "pipe", "pipe"],
    windowsHide: true,
  });
  let log = "";
  let exited = false;
  child.stdout.on("data", (d) => (log = (log + d).slice(-20_000)));
  child.stderr.on("data", (d) => (log = (log + d).slice(-20_000)));
  child.on("exit", () => (exited = true));

  let stopped = false;
  const stop = () => {
    if (stopped) return;
    stopped = true;
    if (!exited) {
      try {
        execFileSync("taskkill", ["/PID", String(child.pid), "/T", "/F"], { stdio: "ignore" });
      } catch {
        child.kill();
      }
    }
    try { rmSync(tmp, { recursive: true, force: true }); } catch {}
  };
  process.once("exit", stop);

  const base = `http://127.0.0.1:${port}`;
  const deadline = Date.now() + startupMs;
  for (;;) {
    if (exited) { stop(); throw new Error(`MV exited during startup\n${log}`); }
    if (Date.now() > deadline) { stop(); throw new Error(`MV /health not up after ${startupMs} ms\n${log}`); }
    try {
      if ((await fetch(`${base}/health`, { signal: AbortSignal.timeout(2000) })).ok) break;
    } catch {}
    await new Promise((r) => setTimeout(r, 300));
  }

  const post = async (folder) => {
    const r = await fetch(`${base}/beatmap/runChecks`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ Folder: folder, IncludeCheckRunDelta: false, CreateSnapshot: false, IncludeCheckTimings: true }),
      signal: AbortSignal.timeout(timeoutMs),
    });
    return { status: r.status, body: await r.text() };
  };
  return { post, stop, log: () => log };
}

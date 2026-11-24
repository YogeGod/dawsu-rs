// Talk to tools/robot/native.ps1: the editor's native file pickers and the Explorer windows it opens.
// One PowerShell runs for the whole robot run; each call sends a line and waits for its answer.
// On a system without PowerShell every call answers { ok: false } and the steps that need a picker
// are skipped.
import { spawn } from "node:child_process";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { createInterface } from "node:readline";

export function startNative() {
  if (process.platform !== "win32") return disabled("native pickers are only driven on Windows");
  const script = join(dirname(fileURLToPath(import.meta.url)), "native.ps1");
  const shell = spawn("powershell.exe", ["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", script], { stdio: ["pipe", "pipe", "inherit"] });
  const lines = createInterface({ input: shell.stdout });
  const waiting = [];
  let ready;
  const started = new Promise(done => (ready = done));
  lines.on("line", line => {
    let answer;
    try { answer = JSON.parse(line); } catch { return; }
    if (answer.ready) return ready(true);
    waiting.shift()?.(answer);
  });
  const stopped = () => {
    ready(false);
    api.available = false;
    api.reason = "the native helper (PowerShell) stopped";
    for (const done of waiting.splice(0)) done({ ok: false, error: "the native helper stopped" });
  };
  shell.on("exit", stopped);
  shell.on("error", stopped);
  shell.stdin.on("error", () => {});
  const send = async command => {
    if (!(await started) || shell.exitCode !== null) return { ok: false, error: "the native helper is not running" };
    return new Promise(done => {
      waiting.push(done);
      shell.stdin.write(command + "\n");
    });
  };
  const list = answer => (answer.windows ?? []).filter(Boolean);
  const api = {
    available: true,
    dialogs: async pid => list(await send(`dialogs ${pid}`)),
    cancel: async pid => (await send(`cancel ${pid}`)).closed ?? 0,
    choose: (pid, path) => send(`choose ${pid} ${path}`),
    explorers: async () => list(await send("explorers")),
    close: hwnd => send(`close ${hwnd}`),
    closeMain: pid => send(`closemain ${pid}`),
    stop: () => { try { shell.stdin.end(); shell.kill(); } catch {} },
  };
  return api;
}

function disabled(reason) {
  const none = async () => [];
  return { available: false, reason, dialogs: none, cancel: async () => 0, choose: async () => ({ ok: false, error: reason }),
    explorers: none, close: async () => ({ ok: false }), closeMain: async () => ({ ok: false, error: reason }), stop: () => {} };
}

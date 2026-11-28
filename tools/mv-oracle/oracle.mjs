// Mapset Verifier oracle: pure helpers (issue keys, before/after diff, case verdicts).
// MV runs as a separate process over loopback; nothing here links to or copies MV code.

/** MV commit the oracle is pinned to (source cache `Naxesss-MapsetVerifier-<short>`). */
export const MV_SHA = "7a4c18dce340d4c373e8ac7d61125f6bf8f9d8df";
export const MV_SHORT = MV_SHA.slice(0, 7);

/** A new issue at one of these levels fails a case; `Minor` and `Info` are only reported. */
export const FAIL_LEVELS = new Set(["Problem", "Warning", "Error"]);

const TIMESTAMP = /\d{2}:\d{2}:\d{3}/g;

/** Keeps `mm:ss:mmm` timestamps and turns every other number into `#`, so a changed measured
 *  value ("20 ms" -> "21 ms") keeps its key while an issue at a new time gets a new one. */
export function normalizeMessage(message) {
  const parts = String(message).split(TIMESTAMP);
  const stamps = String(message).match(TIMESTAMP) ?? [];
  return parts
    // MV formats numbers with the invariant culture, so only "." is a decimal point.
    .map((part, i) => part.replace(/\d+(?:\.\d+)?/g, "#") + (stamps[i] ?? ""))
    .join("");
}

export function issueKey({ check, level, category, message }) {
  return `${check} | ${level} | ${category} | ${normalizeMessage(message)}`;
}

/** Flattens one `POST /beatmap/runChecks` response into issue rows. */
export function flatten(response) {
  const rows = [];
  const checks = response?.checks ?? {};
  for (const cat of [response?.general, ...(response?.difficulties ?? [])]) {
    if (!cat) continue;
    for (const r of cat.checkResults ?? []) {
      const row = {
        check: checks[r.id]?.name ?? `#${r.id}`,
        level: r.level,
        category: cat.category ?? "General",
        message: r.message ?? "",
      };
      rows.push({ ...row, key: issueKey(row) });
    }
  }
  return rows;
}

/** Names of the checks MV ran (needs `IncludeCheckTimings: true`). */
export function checksThatRan(response) {
  const list = response?.checkTimings?.checks ?? [];
  return [...new Set(list.map((t) => t.checkName))].sort();
}

function countByKey(rows) {
  const counts = new Map();
  for (const r of rows) counts.set(r.key, (counts.get(r.key) ?? 0) + 1);
  return counts;
}

/**
 * Issues in `after` beyond those in `before`, counted per key. `allow` holds keys accepted on
 * purpose (each entry cancels one occurrence). Returns `{ failing, minor }`.
 */
export function diffIssues(beforeRows, afterRows, allow = []) {
  const before = countByKey(beforeRows);
  const allowed = countByKey(allow.map((key) => ({ key })));
  const failing = [];
  const minor = [];
  const seen = new Map();
  for (const row of afterRows) {
    const n = (seen.get(row.key) ?? 0) + 1;
    seen.set(row.key, n);
    if (n <= (before.get(row.key) ?? 0) + (allowed.get(row.key) ?? 0)) continue;
    (FAIL_LEVELS.has(row.level) ? failing : minor).push(row);
  }
  return { failing, minor };
}

/**
 * Runs one case. `post(folder)` resolves `{ status, body }` or throws (timeout, connection).
 * Before broken -> SKIP (the input, not dawsu!); after broken -> FAIL; new issues -> FAIL.
 */
export async function evaluateCase({ name, beforeDir, afterDir, post, allow = [] }) {
  const call = async (folder) => {
    try {
      const r = await post(folder);
      if (r.status !== 200) return { error: `HTTP ${r.status}: ${String(r.body).slice(0, 200)}` };
      return { json: typeof r.body === "string" ? JSON.parse(r.body) : r.body };
    } catch (e) {
      return { error: e?.name === "TimeoutError" ? "timed out" : String(e?.message ?? e) };
    }
  };
  const before = await call(beforeDir);
  if (before.error) return { name, result: "SKIP", reason: `before: ${before.error}` };
  const after = await call(afterDir);
  if (after.error) return { name, result: "FAIL", reason: `after: ${after.error}` };
  const { failing, minor } = diffIssues(flatten(before.json), flatten(after.json), allow);
  const ranBefore = checksThatRan(before.json);
  const ranAfter = checksThatRan(after.json);
  return {
    name,
    result: failing.length ? "FAIL" : "PASS",
    reason: failing.length ? `${failing.length} new issue(s)` : "",
    failing,
    minor,
    checksRan: ranAfter,
    checksOnlyBefore: ranBefore.filter((c) => !ranAfter.includes(c)),
    checksOnlyAfter: ranAfter.filter((c) => !ranBefore.includes(c)),
  };
}

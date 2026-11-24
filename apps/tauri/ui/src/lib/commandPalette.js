// Pure logic for dawsu!'s global Ctrl+K Command Palette:
// builds runnable entries from commands.js + active editor_shortcuts, ranks fuzzy matches
// across label, group, and shortcut text, tracks recent commands, and filters enabled state.

export const RECENT_STORAGE_KEY = "dawsu.commandPalette.recent";
export const MAX_RECENT = 8;

export const PALETTE_MENUS = ["file", "edit", "playback", "map", "view", "tool"];

export const GROUP_LABELS = {
  file: "File",
  edit: "Edit",
  playback: "Playback",
  map: "Map",
  view: "View",
  tool: "Tools",
  keyOnly: "Shortcuts",
};

// Menu rows that are segmented choice controls (not single runnable actions) or layout-editor chrome.
const SKIPPED_MENU_IDS = new Set(["editLayout"]);

export function readRecent(storage) {
  if (!storage) return [];
  try {
    const raw = storage.getItem(RECENT_STORAGE_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed.filter(id => typeof id === "string" && id.length > 0).slice(0, MAX_RECENT);
  } catch {
    return [];
  }
}

export function recordRecent(storage, id, existing = null) {
  if (typeof id !== "string" || !id) return existing ? [...existing] : readRecent(storage);
  const base = Array.isArray(existing) ? existing : readRecent(storage);
  const next = [id, ...base.filter(item => item !== id)].slice(0, MAX_RECENT);
  if (storage) {
    try {
      storage.setItem(RECENT_STORAGE_KEY, JSON.stringify(next));
    } catch {
      // Ignore quota / restricted storage errors.
    }
  }
  return next;
}

export function formatShortcutKey(key) {
  if (typeof key === "string") return key.trim();
  if (key && typeof key === "object") {
    if (typeof key.label === "string") return key.label.trim();
    if (typeof key.key === "string") return key.key.trim();
  }
  return "";
}

export function normalizeShortcutMap(shortcuts) {
  const map = new Map();
  if (!shortcuts) return map;
  if (shortcuts instanceof Map) {
    for (const [id, value] of shortcuts.entries()) {
      if (Array.isArray(value)) {
        map.set(id, { label: "", keys: value.map(formatShortcutKey).filter(Boolean) });
      } else if (value && typeof value === "object") {
        map.set(id, {
          label: typeof value.label === "string" ? value.label : "",
          keys: (value.keys ?? []).map(formatShortcutKey).filter(Boolean),
        });
      }
    }
    return map;
  }
  const entries = Array.isArray(shortcuts) ? shortcuts : shortcuts.entries;
  if (Array.isArray(entries)) {
    for (const row of entries) {
      if (Array.isArray(row)) {
        const [id, label, keys] = row;
        if (typeof id === "string") {
          map.set(id, {
            label: typeof label === "string" ? label : "",
            keys: (keys ?? []).map(formatShortcutKey).filter(Boolean),
          });
        }
      } else if (row && typeof row.id === "string") {
        map.set(row.id, {
          label: typeof row.label === "string" ? row.label : "",
          keys: (row.keys ?? []).map(formatShortcutKey).filter(Boolean),
        });
      }
    }
  }
  return map;
}

export function evaluateEnabledRule(ruleName, { scene = null, view = null } = {}) {
  const busy = Boolean(view?.busy || view?.fault);
  const hasScene = Boolean(scene);
  const canEdit = hasScene && !busy;
  switch (ruleName ?? "edit") {
    case "idle":
      return !busy;
    case "document":
      return hasScene && !busy;
    case "selection":
      return canEdit && Boolean(scene?.selection?.count);
    case "range":
      return canEdit && Boolean(scene?.range);
    case "placement":
      return canEdit && scene?.placement != null;
    case "undo":
      return canEdit && scene?.historyPosition !== 0;
    case "redo":
      return canEdit && scene?.historyPosition !== scene?.history?.length;
    case "deleteLane":
      return canEdit && Boolean(scene?.lanes?.[view?.activeLane ?? 0]?.canDelete);
    case "originalSnapshot":
      return hasScene && Boolean(view?.hasDocument && view?.hasOriginalSnapshot) && !busy;
    case "undoRevert":
      return hasScene && Boolean(view?.hasDocument && view?.canUndoRevert) && !busy;
    case "never":
      return false;
    case "edit":
    default:
      return canEdit;
  }
}

export function buildEntries({
  items = [],
  menus = {},
  keyOnly = [],
  shortcuts = null,
  scene = null,
  view = null,
  isEnabled = null,
  isChecked = null,
} = {}) {
  const byId = new Map(items.map(item => [item.id, item]));
  const shortcutMap = normalizeShortcutMap(shortcuts);
  const entries = [];
  const seenCommands = new Set();

  const historyStep = offset => scene?.history?.[scene.historyPosition + offset];

  for (const menuKey of PALETTE_MENUS) {
    const group = GROUP_LABELS[menuKey] ?? menuKey;
    const rowIds = menus[menuKey] ?? [];
    for (const id of rowIds) {
      if (id === "-" || SKIPPED_MENU_IDS.has(id)) continue;
      const item = byId.get(id);
      if (!item || item.choices) continue;

      const shortcutInfo = item.command ? shortcutMap.get(item.command) : null;
      const keys = shortcutInfo?.keys ?? [];
      if (item.command) seenCommands.add(item.command);

      const historyAction = item.history === undefined ? null : historyStep(item.history);
      const label = historyAction ? `${item.label} · ${historyAction}` : item.label;
      const enabled = typeof isEnabled === "function"
        ? Boolean(isEnabled(item))
        : evaluateEnabledRule(item.enabled, { scene, view });
      const checked = typeof isChecked === "function" ? isChecked(item) : undefined;

      entries.push({
        id: item.id,
        label,
        group,
        icon: item.icon ?? null,
        command: item.command ?? null,
        keys,
        shortcutText: keys[0] ?? "",
        enabled,
        checked,
        source: "menu",
        item,
      });
    }
  }

  for (const commandId of keyOnly) {
    if (seenCommands.has(commandId)) continue;
    const shortcutInfo = shortcutMap.get(commandId);
    if (!shortcutInfo?.label) continue;
    const keys = shortcutInfo.keys ?? [];
    const enabled = Boolean(scene) && !Boolean(view?.busy || view?.fault);
    entries.push({
      id: `cmd:${commandId}`,
      label: shortcutInfo.label,
      group: GROUP_LABELS.keyOnly,
      icon: null,
      command: commandId,
      keys,
      shortcutText: keys[0] ?? "",
      enabled,
      checked: undefined,
      source: "keyOnly",
      item: null,
    });
  }

  return entries;
}

function isWordBoundary(text, index) {
  if (index === 0) return true;
  const prev = text[index - 1];
  return prev === " " || prev === "/" || prev === "-" || prev === "·" || prev === "+" || prev === "…";
}

function fuzzySubsequenceScore(text, token) {
  let ti = 0;
  let score = 0;
  let consecutive = 0;
  let firstMatch = -1;
  for (let i = 0; i < text.length && ti < token.length; i++) {
    if (text[i] === token[ti]) {
      if (firstMatch === -1) firstMatch = i;
      consecutive++;
      score += 10 + consecutive * 6 + (isWordBoundary(text, i) ? 14 : 0);
      ti++;
    } else {
      consecutive = 0;
    }
  }
  if (ti < token.length) return 0;
  // Prefer compact matches starting near the beginning.
  return Math.max(40, Math.min(280, score - firstMatch * 2));
}

function scoreToken(entry, token) {
  const label = entry.label.toLowerCase();
  const group = entry.group.toLowerCase();
  const shortcuts = (entry.keys ?? []).join(" ").toLowerCase();
  const compactShortcut = shortcuts.replace(/[\s+]/g, "");
  const compactToken = token.replace(/[\s+]/g, "");

  if (label === token) return 1000;
  if (label.startsWith(token)) return 860 - Math.min(40, label.length);

  const labelIndex = label.indexOf(token);
  if (labelIndex >= 0) {
    return isWordBoundary(label, labelIndex) ? 720 - labelIndex : 560 - Math.min(60, labelIndex);
  }

  if (shortcuts && (shortcuts.includes(token) || (compactToken.length >= 2 && compactShortcut.includes(compactToken)))) {
    return shortcuts.startsWith(token) ? 460 : 410;
  }

  if (group === token) return 380;
  if (group.startsWith(token)) return 340;
  if (group.includes(token)) return 300;

  const sub = fuzzySubsequenceScore(label, token);
  if (sub > 0) return sub;

  const combined = `${group} ${label}`;
  const combinedSub = fuzzySubsequenceScore(combined, token);
  if (combinedSub > 0) return Math.round(combinedSub * 0.75);

  return 0;
}

export function scoreMatch(entry, rawQuery) {
  const query = String(rawQuery ?? "").trim().toLowerCase();
  if (!query) return 1;

  // Fast full-query boost for exact or prefix label matches before token splitting.
  const label = entry.label.toLowerCase();
  let bonus = 0;
  if (label === query) bonus = 400;
  else if (label.startsWith(query)) bonus = 220;
  else if (label.includes(query)) bonus = 100;

  const tokens = query.split(/\s+/).filter(Boolean);
  let total = 0;
  for (const token of tokens) {
    const tokenScore = scoreToken(entry, token);
    if (tokenScore <= 0) return 0;
    total += tokenScore;
  }
  return Math.round(total / tokens.length) + bonus;
}

export function filterAndRank(entries, rawQuery = "", recentIds = []) {
  const query = String(rawQuery ?? "").trim();
  const recentRank = new Map(recentIds.map((id, index) => [id, index]));

  if (!query) {
    const indexed = entries.map((entry, index) => ({
      ...entry,
      recent: recentRank.has(entry.id),
      _recentPos: recentRank.has(entry.id) ? recentRank.get(entry.id) : Infinity,
      _index: index,
    }));
    indexed.sort((a, b) => {
      if (a._recentPos !== b._recentPos) return a._recentPos - b._recentPos;
      return a._index - b._index;
    });
    return indexed.map(({ _recentPos, _index, ...entry }) => entry);
  }

  const matched = [];
  for (let index = 0; index < entries.length; index++) {
    const entry = entries[index];
    const score = scoreMatch(entry, query);
    if (score <= 0) continue;
    matched.push({
      ...entry,
      recent: recentRank.has(entry.id),
      _score: score,
      _recentPos: recentRank.has(entry.id) ? recentRank.get(entry.id) : Infinity,
      _index: index,
    });
  }

  matched.sort((a, b) => {
    if (b._score !== a._score) return b._score - a._score;
    if (a.enabled !== b.enabled) return a.enabled ? -1 : 1;
    if (a._recentPos !== b._recentPos) return a._recentPos - b._recentPos;
    return a._index - b._index;
  });

  return matched.map(({ _score, _recentPos, _index, ...entry }) => entry);
}

export function firstEnabledIndex(entries) {
  return entries.findIndex(entry => entry.enabled);
}

export function nextEnabledIndex(entries, currentIndex, delta) {
  const count = entries.length;
  if (!count) return -1;
  const step = delta >= 0 ? 1 : -1;
  let start = currentIndex;
  if (start < 0 || start >= count) {
    start = step > 0 ? -1 : count;
  }
  for (let i = 1; i <= count; i++) {
    const candidate = ((start + step * i) % count + count) % count;
    if (entries[candidate]?.enabled) return candidate;
  }
  return -1;
}

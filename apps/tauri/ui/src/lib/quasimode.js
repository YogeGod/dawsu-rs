// Hold-key tool quasimode state machine (DAW-style temporary tool switching + dual tap/hold "4").
// Pure ES module with dependency injection so browser and Node unit tests share exact behavior.

export const QUASIMODE_HOLD_MS = 180;
export const MORE_TOOLS_KEY = "4";

export const TOOL_COMMAND_TO_NAME = {
  toolSelect: "Select",
  toolDraw: "Draw",
  toolErase: "Erase",
  toolMute: "Mute",
  toolGrab: "Grab",
  toolRange: "Range",
  toolRamp: "Ramp",
};

// Mirrors crates/dawsu-editor/src/tools.rs:221-249 and ipc.js:160-167 (Range is unbound).
export const DEFAULT_DAW_TOOL_KEYS = {
  "1": "Select",
  "2": "Grab",
  "3": "Draw",
  "5": "Mute",
  "6": "Ramp",
};

export const DEFAULT_OSU_TOOL_KEYS = {
  "1": "Select",
  "2": "Draw",
  "5": "Mute",
  "6": "Ramp",
};

function singlePlainKey(keyLabel) {
  if (typeof keyLabel !== "string") return null;
  const trimmed = keyLabel.trim();
  if (!trimmed || trimmed.includes("+")) return null;
  return trimmed.length === 1 ? trimmed.toLowerCase() : null;
}

export function buildToolKeyMap(shortcuts = null, keyPreset = "DAW") {
  const entries = Array.isArray(shortcuts) ? shortcuts : shortcuts?.entries;
  if (!Array.isArray(entries)) {
    return keyPreset === "osu!" ? { ...DEFAULT_OSU_TOOL_KEYS } : { ...DEFAULT_DAW_TOOL_KEYS };
  }
  const map = {};
  for (const row of entries) {
    const id = Array.isArray(row) ? row[0] : row?.id;
    const keys = Array.isArray(row) ? row[2] : row?.keys;
    const toolName = TOOL_COMMAND_TO_NAME[id];
    if (!toolName || !Array.isArray(keys)) continue;
    for (const raw of keys) {
      const k = singlePlainKey(raw);
      if (k && k !== MORE_TOOLS_KEY) map[k] = toolName;
    }
  }
  return map;
}

export function createQuasimodeController({
  holdMs = QUASIMODE_HOLD_MS,
  keyPreset = "DAW",
  getTool = () => "Select",
  setTool = () => {},
  isDragging = () => false,
  openMoreTools = () => {},
  setTimeoutFn = globalThis.setTimeout?.bind(globalThis),
  clearTimeoutFn = globalThis.clearTimeout?.bind(globalThis),
  now = () => Date.now(),
} = {}) {
  let shortcutsCache = null;
  let presetCache = keyPreset;
  let keyMap = buildToolKeyMap(null, presetCache);
  let session = null;

  function clearTimer(target) {
    if (target?.timer != null && clearTimeoutFn) {
      clearTimeoutFn(target.timer);
      target.timer = null;
    }
  }

  function restorePrior(priorTool) {
    if (priorTool && getTool() !== priorTool) setTool(priorTool);
  }

  return {
    setShortcuts(shortcuts) {
      shortcutsCache = shortcuts ?? null;
      if (shortcuts?.preset) presetCache = shortcuts.preset;
      keyMap = buildToolKeyMap(shortcutsCache, presetCache);
    },

    setPreset(nextPreset) {
      presetCache = nextPreset || "DAW";
      if (!shortcutsCache) keyMap = buildToolKeyMap(null, presetCache);
    },

    getKeyMap() {
      return { ...keyMap };
    },

    isActive() {
      return Boolean(session);
    },

    activeState() {
      return session ? { ...session } : null;
    },

    handleKeyDown({ key, ctrl = false, shift = false, alt = false, repeat = false } = {}) {
      if (ctrl || shift || alt || isDragging()) return false;
      const normalized = typeof key === "string" ? key.toLowerCase() : "";
      if (!normalized) return false;
      if (session && session.key === normalized) {
        // Consume repeats of an active quasimode key without retriggering IPC.
        return true;
      }
      if (repeat) return false;

      const dual = normalized === MORE_TOOLS_KEY;
      const tool = dual ? "Erase" : keyMap[normalized];
      if (!tool) return false;

      if (session) {
        clearTimer(session);
        if (session.held || session.strokeStarted) restorePrior(session.priorTool);
        session = null;
      }

      const priorTool = getTool() ?? "Select";
      const next = {
        key: normalized,
        tool,
        dual,
        priorTool,
        held: false,
        strokeStarted: false,
        pendingRestore: false,
        timer: null,
        startedAt: now(),
      };
      next.timer = setTimeoutFn
        ? setTimeoutFn(() => {
            if (session !== next) return;
            next.held = true;
            if (next.dual && getTool() !== next.tool) setTool(next.tool);
          }, holdMs)
        : null;
      session = next;

      if (!dual && priorTool !== tool) {
        setTool(tool);
      }
      return true;
    },

    notifyStrokeStart() {
      if (!session) return false;
      session.strokeStarted = true;
      session.held = true;
      clearTimer(session);
      if (session.dual && getTool() !== session.tool) {
        setTool(session.tool);
      }
      return true;
    },

    notifyStrokeEnd() {
      if (!session?.pendingRestore) return false;
      const priorTool = session.priorTool;
      clearTimer(session);
      session = null;
      restorePrior(priorTool);
      return true;
    },

    handleKeyUp({ key } = {}) {
      const normalized = typeof key === "string" ? key.toLowerCase() : "";
      if (!session || session.key !== normalized) return false;
      const current = session;
      clearTimer(current);
      if (!current.held && now() - current.startedAt >= holdMs) {
        current.held = true;
      }
      if (!current.held && !current.strokeStarted) {
        session = null;
        if (current.dual) openMoreTools();
        return true;
      }
      if (isDragging()) {
        current.pendingRestore = true;
        return true;
      }
      session = null;
      restorePrior(current.priorTool);
      return true;
    },

    cancel(restore = true) {
      if (!session) return false;
      const current = session;
      clearTimer(current);
      session = null;
      if (restore && (current.held || current.strokeStarted)) {
        restorePrior(current.priorTool);
      }
      return true;
    },
  };
}

// The plain key bound to `tool` in a key map, or null when none is.
export function toolKeyFor(keyMap, tool) {
  return Object.keys(keyMap ?? {}).find(key => keyMap[key] === tool) ?? null;
}

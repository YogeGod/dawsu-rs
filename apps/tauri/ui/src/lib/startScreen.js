// Pure helpers for the editor's Start Screen (#home).

// "Artist - Title (Creator) [Difficulty].osu" gives the map name and "[Difficulty]"; other names show whole.
export function recentMapName(path) {
  const parts = String(path).split(/[\\/]/);
  const base = parts[parts.length - 1] ?? "";
  const file = base.replace(/\.(osu|osz)$/i, "");
  const match = /^(.+?)\s*\([^()]*\)\s*(\[[^\]]*\])$/.exec(file);
  return match ? [match[1], match[2]] : [file, ""];
}

export function isHitsoundVersion(version) {
  const v = String(version ?? "").trim().toLowerCase();
  return Boolean(
    v && (v.includes("hitsound") || v === "hs" || v.endsWith(" hs") || v.includes("[hs]"))
  );
}

export function songsSourceTooltip(source) {
  switch (source) {
    case "osu!.db":
      return "Indexed from osu! database";
    case "cache":
      return "Loaded from local Songs cache";
    case "folder-scan":
      return "Scanned from Songs folder";
    default:
      return "";
  }
}

export function recentItems(view) {
  const metaByPath = new Map((view?.recentMeta ?? []).map(entry => [entry.path, entry]));
  return (view?.recent ?? []).map(path => {
    const [name, difficulty] = recentMapName(path);
    const meta = metaByPath.get(path);
    if (!meta) return { path, name, difficulty };
    return {
      path,
      name,
      difficulty,
      backgroundUrl: meta.background || "",
      isHs: Boolean(meta.isHs),
    };
  });
}

export function recoveryState(view, scene) {
  if (scene || view?.hasDocument || !view?.recovery) {
    return { visible: false, error: "", offers: [] };
  }
  const error = view.recovery.error ?? "";
  const offers = (view.recovery.offers ?? []).map(offer => ({
    id: offer.id,
    name: offer.mapset + " [" + offer.difficulty + "]",
    createdText: new Date(offer.created_ms).toLocaleString(),
  }));
  return {
    visible: Boolean(error || offers.length),
    error,
    offers,
  };
}

export const SETUP_CHECKLIST_DISMISSED_KEY = "dawsu.setupChecklistDismissed";
export const PRACTICE_TRIED_KEY = "dawsu.practiceTried";

export function isSetupChecklistDismissed(storage) {
  try {
    return storage?.getItem?.(SETUP_CHECKLIST_DISMISSED_KEY) === "1";
  } catch {
    return false;
  }
}

export function dismissSetupChecklist(storage) {
  try {
    storage?.setItem?.(SETUP_CHECKLIST_DISMISSED_KEY, "1");
  } catch {
    // Ignore storage errors in restricted environments.
  }
}

export function isPracticeTried(storage) {
  try {
    return storage?.getItem?.(PRACTICE_TRIED_KEY) === "1";
  } catch {
    return false;
  }
}

export function markPracticeTried(storage) {
  try {
    storage?.setItem?.(PRACTICE_TRIED_KEY, "1");
  } catch {
    // Ignore storage errors in restricted environments.
  }
}

export function isAudioOutputReady(outputDevice) {
  const name = String(outputDevice ?? "").trim();
  return Boolean(name) && name.toLowerCase() !== "audio unavailable";
}

export function setupChecklistState(view, dismissed = false, { practiceTried = false } = {}) {
  const hasAudioOutput = isAudioOutputReady(view?.outputDevice);
  const hasSongsFolder = Boolean(view?.songsFolder && String(view.songsFolder).trim());
  const hasShortcutStyle = Boolean(view?.keyPreset) && !view?.firstLaunch;
  const hasFirstMap =
    (view?.recent?.length ?? 0) > 0 || Boolean(view?.hasDocument) || Boolean(practiceTried);
  const steps = [
    {
      id: "audio",
      label: "Audio",
      done: hasAudioOutput,
      stateLabel: hasAudioOutput ? "Ready" : "Set output",
      detail: hasAudioOutput ? `Output: ${view.outputDevice}` : "Choose an audio output device",
      action: "audio",
    },
    {
      id: "songs-folder",
      label: "Songs folder",
      done: hasSongsFolder,
      stateLabel: hasSongsFolder ? "Set" : "Choose",
      detail: hasSongsFolder ? `Songs: ${view.songsFolder}` : "Point to your osu! Songs folder",
      action: "folders",
    },
    {
      id: "shortcut-style",
      label: "Shortcuts",
      done: hasShortcutStyle,
      stateLabel: hasShortcutStyle ? String(view.keyPreset) : "Choose",
      detail: hasShortcutStyle ? `Preset: ${view.keyPreset}` : "Pick osu! or DAW-like keys",
      action: "shortcuts",
    },
    {
      id: "first-map",
      label: "First map",
      done: hasFirstMap,
      stateLabel: hasFirstMap ? "Ready" : "Try practice",
      detail: hasFirstMap ? "Map opened" : "Open a map or try the bundled practice mapset",
      action: "tryPractice",
    },
  ];
  const completedCount = steps.filter(step => step.done).length;
  const allDone = completedCount === steps.length;
  return {
    visible: !dismissed && !allDone,
    allDone,
    completedCount,
    totalCount: steps.length,
    steps,
  };
}

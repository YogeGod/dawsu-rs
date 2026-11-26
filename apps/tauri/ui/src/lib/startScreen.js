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

export function setupChecklistState(view, dismissed = false) {
  const hasSongsFolder = Boolean(view?.songsFolder && String(view.songsFolder).trim());
  const hasShortcutStyle = Boolean(view?.keyPreset) && !view?.firstLaunch;
  const hasOpenedMap = (view?.recent?.length ?? 0) > 0;
  const steps = [
    {
      id: "songs-folder",
      label: "Set your Songs folder",
      detail: "Search and open maps straight from osu! Songs",
      done: hasSongsFolder,
      action: "folders",
      actionLabel: "Open Folders",
    },
    {
      id: "shortcut-style",
      label: "Pick your shortcut style",
      detail: "Choose DAW-like or osu! keyboard shortcuts",
      done: hasShortcutStyle,
      action: "shortcuts",
      actionLabel: "Open Shortcuts",
    },
    {
      id: "first-map",
      label: "Open your first map",
      detail: "Open an .osu difficulty or .osz mapset",
      done: hasOpenedMap,
      action: "open",
      actionLabel: "Open map",
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

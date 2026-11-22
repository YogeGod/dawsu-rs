// Pure helpers for the editor's Start Screen (#home).

// "Artist - Title (Creator) [Difficulty].osu" gives the map name and "[Difficulty]"; other names show whole.
export function recentMapName(path) {
  const parts = String(path).split(/[\\/]/);
  const base = parts[parts.length - 1] ?? "";
  const file = base.replace(/\.(osu|osz)$/i, "");
  const match = /^(.+?)\s*\([^()]*\)\s*(\[[^\]]*\])$/.exec(file);
  return match ? [match[1], match[2]] : [file, ""];
}

export function recentItems(view) {
  const metaByPath = new Map((view?.recentMeta ?? []).map(entry => [entry.path, entry]));
  const convert = globalThis.window?.__TAURI__?.core?.convertFileSrc;
  return (view?.recent ?? []).map(path => {
    const [name, difficulty] = recentMapName(path);
    const meta = metaByPath.get(path);
    if (!meta) return { path, name, difficulty };
    const backgroundUrl = meta.background && convert ? convert(meta.background) : (meta.background || "");
    return { path, name, difficulty, backgroundUrl, isHs: Boolean(meta.isHs) };
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

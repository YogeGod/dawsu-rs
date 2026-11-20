// Plain logic for the Export window, tested under node without a webview.

// What the Export window starts with for a map it has no stored choices for.
export const defaultChoices = Object.freeze({ leftOut: [], matchWithin: 5, copyVolumes: true });

// The old Copy dialog's defaults for every option this slice does not show yet, so Export copies
// exactly as Copy did with Match within and Copy volumes set.
export function copyOptions({ matchWithin, copyVolumes }) {
  return {
    mode: "overwrite_everything",
    leniency: matchWithin,
    start_index: 100,
    never_silent_hitnormals: false,
    copy_hitsounds: true,
    copy_body_hitsounds: true,
    copy_to_slider_ticks: false,
    copy_to_slider_slides: false,
    copy_sample_sets: true,
    copy_volumes: copyVolumes,
    always_preserve_5_volume: true,
    copy_storyboarded_samples: false,
    ignore_hitsound_satisfied_samples: true,
    ignore_whenever_hitsound: false,
    mute_slider_ends: false,
    beat_divisors: [1, 4, 3, 8, 6, 16, 12],
    muted_divisors: [4, 3, 8, 6, 16, 12],
    min_length: 0.5,
    muted_index: null,
    muted_sample_set: null,
  };
}

export const fileName = path => String(path).split(/[\\/]/).pop();

// "Artist - Title (Mapper) [Insane].osu" reads as "Insane", like osu! names a difficulty.
export function difficultyName(path) {
  const name = fileName(path);
  return name.match(/\[([^\]]*)\]\.osu$/i)?.[1] || name.replace(/\.osu$/i, "");
}

export function choicesFrom(view) {
  const stored = view?.choices;
  if (!stored) return { ...defaultChoices, leftOut: [] };
  return {
    leftOut: Array.isArray(stored.leftOut) ? [...stored.leftOut] : [],
    matchWithin: Number.isFinite(stored.matchWithin) ? stored.matchWithin : defaultChoices.matchWithin,
    copyVolumes: typeof stored.copyVolumes === "boolean" ? stored.copyVolumes : defaultChoices.copyVolumes,
  };
}

export const included = (targets, choices) =>
  (targets ?? []).filter(path => !choices.leftOut.includes(fileName(path)));

// Match within is in ms, as Copy's timing tolerance was: 0 to 1000.
export const matchWithinValid = value => Number.isFinite(value) && value >= 0 && value <= 1000;

export function exportLabel(count) {
  if (count === 0) return "Export";
  return count === 1 ? "Export 1 difficulty" : `Export ${count} difficulties`;
}

// Everything a preview depends on that this window chooses. A preview sent with another key is old.
export function optionsKey(view, choices) {
  const targets = included(view?.targets, choices);
  return JSON.stringify([view?.source?.path ?? null, targets, choices.matchWithin, choices.copyVolumes]);
}

export function previewRequest(view, choices) {
  return {
    options: {
      source: view.source.path,
      targets: included(view.targets, choices),
      options: copyOptions(choices),
    },
  };
}

// sent is { id, key } of the last preview this window asked for, or null while an option changed
// and the new preview is not asked for yet. Export needs this window's own finished preview, of the
// map as it is now: a page reload or another window's request never enables it.
export function exportState(view, sent, choices) {
  const preview = view?.preview;
  const source = view?.source;
  const mine = !!(preview && sent && preview.id === sent.id && sent.key === optionsKey(view, choices));
  const stale = mine && (preview.stale || preview.version !== source?.version || preview.source !== source?.path);
  const count = included(view?.targets, choices).length;
  const canExport = mine && !stale && count > 0 && !!source && !view.busy && !view.writing
    && matchWithinValid(choices.matchWithin);
  return { canExport, stale, count, preview: mine && !stale ? preview : null };
}

// The counts of one target, from the preview Copy planned. Zero counts that need no action stay hidden.
export function targetCounts(preview, path) {
  const counts = preview?.targets?.find(item => item.path === path);
  if (!counts) return null;
  const items = [
    { tone: "matched", text: `${counts.matched} matched` },
    { tone: "unmatched", text: `${counts.unmatched} unmatched` },
  ];
  if (counts.collisions) items.push({ tone: "replaced", text: `${counts.collisions} replaced` });
  if (counts.keptFromSilent) items.push({ tone: "kept", text: `${counts.keptFromSilent} kept to avoid silence` });
  if (counts.muted) items.push({ tone: "muted", text: `${counts.muted} ends muted` });
  return { changed: counts.changed, items };
}

// Poll faster while something is running, so progress and the result show without delay.
export const pollDelay = view => (view?.busy || view?.previewing || view?.writing ? 100 : 250);

// A lane's chosen colour. Lanes are plain grey unless the mapper picked one, so only picked ones show.
export function laneSwatches(source) {
  const colours = source?.laneColours ?? {};
  return (source?.lanes ?? [])
    .map(lane => colours[lane.colourKey])
    .filter(colour => typeof colour === "string" && /^#[\da-f]{6}$/i.test(colour));
}

// With Tauri's drop handler off, WebView2 would open a dropped file in place of the page.
export function blockDrops(target) {
  const stop = event => event.preventDefault();
  target.addEventListener("dragover", stop);
  target.addEventListener("drop", stop);
  return () => {
    target.removeEventListener("dragover", stop);
    target.removeEventListener("drop", stop);
  };
}

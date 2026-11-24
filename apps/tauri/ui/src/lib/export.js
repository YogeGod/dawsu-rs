// Plain logic for the Export window, tested under node without a webview.

// What the Export window starts with for a map it has no stored choices for. The base hitsound
// difficulty is a new file in the mapset, so it is written only once the mapper turns it on.
export const defaultChoices = Object.freeze({
  leftOut: [], matchWithin: 5, copyVolumes: true, base: false, sliderAudio: false, laterWins: false,
});

// Generate's name for the difficulty it makes (HITSOUNDS_VERSION in dawsu-hitsounding).
export const baseName = "Hitsounds";

// The old Copy dialog's defaults for every option this slice does not show yet, so Export copies
// exactly as Copy did with Match within and Copy volumes set.
export function copyOptions({ matchWithin, copyVolumes, laterWins = false }) {
  return {
    mode: "overwrite_everything",
    leniency: matchWithin,
    collision_policy: laterWins ? "later_wins" : "merge",
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
    base: typeof stored.base === "boolean" ? stored.base : defaultChoices.base,
    sliderAudio: typeof stored.sliderAudio === "boolean" ? stored.sliderAudio : defaultChoices.sliderAudio,
    laterWins: typeof stored.laterWins === "boolean" ? stored.laterWins : defaultChoices.laterWins,
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
  return JSON.stringify([view?.source?.path ?? null, targets, choices.matchWithin, choices.copyVolumes,
    choices.base, choices.sliderAudio, choices.laterWins === true]);
}

export function previewRequest(view, choices) {
  return {
    options: {
      source: view.source.path,
      targets: included(view.targets, choices),
      options: copyOptions(choices),
      base: choices.base === true,
      sliderAudio: choices.sliderAudio === true,
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
  // The base hitsound difficulty is one more difficulty written.
  const count = included(view?.targets, choices).length + (choices.base ? 1 : 0);
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

// What the base row says once previewed: Generate makes a new difficulty or replaces its earlier one.
export function baseStatus(preview) {
  const base = preview?.base;
  if (!base) return null;
  return base.replaces ? "Replaces earlier, backed up" : "New difficulty";
}

// Groups consecutive changed `.osu` lines under their active section header (e.g. `[TimingPoints]`).
export function diffView(diff) {
  if (!diff || (!diff.added && !diff.removed)) return null;
  const groups = [];
  for (const entry of diff.entries ?? []) {
    const section = entry.section ?? null;
    const last = groups[groups.length - 1];
    if (last && last.section === section) {
      last.entries.push(entry);
    } else {
      groups.push({ section, entries: [entry] });
    }
  }
  return {
    added: diff.added ?? 0,
    removed: diff.removed ?? 0,
    truncated: diff.truncated ?? 0,
    groups,
  };
}

export const baseDiff = preview => diffView(preview?.base?.diff ?? null);

export const targetDiff = (preview, path) =>
  diffView(preview?.targets?.find(item => item.path === path)?.diff ?? null);

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

// Stored choices are taken once per map, never on every poll, so a click is never undone.
// Answers the map now adopted, and the choices to start from, or null to keep the window's own.
export function adoptChoices(adoptedFor, view) {
  const path = view?.source?.path;
  if (path === adoptedFor) return { adoptedFor, choices: null };
  return { adoptedFor: path, choices: choicesFrom(view) };
}

// run is { id } of this window's Export press; id is null until export_run answers with it.
// Only the result with that id is this run's: an earlier run's error never ends it.
export function runOutcome(run, view) {
  if (!run) return null;
  if (run.id === null || view?.writeId !== run.id || view.writing) return "pending";
  if (view.writeError) return "failed";
  return view.written !== null && view.written !== undefined ? "written" : "pending";
}

// What a poll's answer asks of the window: close after its own write, or preview again after a
// failed one (the write used up the preview, and nothing else would ask for a new one).
export function afterPoll(run, view) {
  const outcome = runOutcome(run, view);
  if (outcome === "written") return { run: null, close: true, preview: false };
  if (outcome === "failed") return { run: null, close: false, preview: true };
  return { run, close: false, preview: false };
}

// The action bar's message. again: show "Preview again" (or Refresh) beside it. writeError is the
// failed write of this window's own last run: an older one, or one from before a reload, is not shown.
export function statusLine({ view, pollError, actionError, writeError, stale, running, problems }) {
  if (pollError) return { tone: "error", text: pollError, again: false };
  if (actionError) return { tone: "error", text: actionError, again: actionError.endsWith("Preview again.") };
  if (writeError) return { tone: "error", text: writeError, again: false };
  if (view?.error) return { tone: "error", text: view.error, again: true };
  if (stale) {
    const text = view?.preview?.staleReason === "mergeSettings" ? "Merge settings changed since preview" : "Map changed since preview";
    return { tone: "stale", text, again: true };
  }
  if (running || view?.writing) return { tone: "busy", text: "Writing…", again: false };
  if (view?.previewing) {
    const progress = view.progress ? ` ${view.progress.done}/${view.progress.total}` : "";
    return { tone: "busy", text: "Previewing" + progress, again: false };
  }
  if (problems?.length) return { tone: "warning", text: problems[0].message, count: problems.length, again: false };
  return null;
}

// The footer's "N warnings" button and the list it opens: the messages while open, none while
// closed or once no warning is left.
export const warningsLabel = count => count + (count === 1 ? " warning" : " warnings");
export const openWarnings = (open, line, problems) => (open && line?.count ? problems.map(problem => problem.message) : []);

// This window's own failed write, once its last run's result is in.
export const ownWriteError = (lastRun, view) =>
  lastRun !== null && lastRun !== undefined && view?.writeId === lastRun && !view.writing ? view.writeError ?? null : null;

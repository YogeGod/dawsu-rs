// Pure comparison helpers to isolate ~85 ms playhead/time updates from unrelated Svelte parts.

function sameStrings(a, b) {
  const left = a ?? [];
  const right = b ?? [];
  if (left === right) return true;
  if (left.length !== right.length) return false;
  for (let i = 0; i < left.length; i++) if (left[i] !== right[i]) return false;
  return true;
}

function sameProblems(a, b) {
  const left = a ?? [];
  const right = b ?? [];
  if (left === right) return true;
  if (left.length !== right.length) return false;
  for (let i = 0; i < left.length; i++) {
    if (left[i].time !== right[i].time || left[i].message !== right[i].message ||
        left[i].target !== right[i].target || left[i].timeline !== right[i].timeline) return false;
  }
  return true;
}

function sameRecovery(a, b) {
  if (a === b) return true;
  if (!a || !b) return !a && !b;
  if ((a.error ?? null) !== (b.error ?? null)) return false;
  const left = a.offers ?? [];
  const right = b.offers ?? [];
  if (left.length !== right.length) return false;
  for (let i = 0; i < left.length; i++) {
    if (left[i].id !== right[i].id || left[i].created_ms !== right[i].created_ms ||
        left[i].mapset !== right[i].mapset || left[i].difficulty !== right[i].difficulty) return false;
  }
  return true;
}

// Drum Map Setup's switches and rows; the view sends a new object on every update.
function sameDrumView(a, b) {
  if (a === b) return true;
  if (!a || !b) return false;
  if (!!a.enabled !== !!b.enabled || (a.name ?? null) !== (b.name ?? null) ||
    !!a.autoDetect !== !!b.autoDetect || !!a.hideUnassigned !== !!b.hideUnassigned) return false;
  const ra = a.rows ?? [], rb = b.rows ?? [];
  return ra.length === rb.length && ra.every((row, i) => row.name === rb[i].name &&
    (row.laneKey ?? null) === (rb[i].laneKey ?? null) && !!row.assigned === !!rb[i].assigned);
}

export function sameUiView(a, b) {
  if (a === b) return true;
  if (!a || !b) return false;
  return (
    !!a.busy === !!b.busy &&
    (a.fault ?? null) === (b.fault ?? null) &&
    !!a.playing === !!b.playing &&
    !!a.cycle === !!b.cycle &&
    Object.is(a.duration, b.duration) &&
    Object.is(a.speed, b.speed) &&
    !!a.audition === !!b.audition &&
    (a.tool ?? null) === (b.tool ?? null) &&
    !!a.follow === !!b.follow &&
    !!a.returnOnStop === !!b.returnOnStop &&
    (a.panels?.controller === false) === (b.panels?.controller === false) &&
    (a.panels?.browser === false) === (b.panels?.browser === false) &&
    (a.panels?.inspector === false) === (b.panels?.inspector === false) &&
    (a.panels?.laneNames === false) === (b.panels?.laneNames === false) &&
    Object.is(a.song, b.song) &&
    Object.is(a.main, b.main) &&
    Object.is(a.hitsounds, b.hitsounds) &&
    (a.outputDevice || "") === (b.outputDevice || "") &&
    !!a.songMuted === !!b.songMuted &&
    !!a.songSolo === !!b.songSolo &&
    !!a.hitsoundsMuted === !!b.hitsoundsMuted &&
    !!a.hitsoundsSolo === !!b.hitsoundsSolo &&
    (a.status || "") === (b.status || "") &&
    !!a.showHome === !!b.showHome &&
    !!a.hasDocument === !!b.hasDocument &&
    !!a.previewWindow === !!b.previewWindow &&
    !!a.objectLines === !!b.objectLines &&
    // Preferences and the drum lane column read these.
    !!a.hsMode === !!b.hsMode &&
    (a.songsFolder || "") === (b.songsFolder || "") &&
    (a.hitsoundLibrary || "") === (b.hitsoundLibrary || "") &&
    Object.is(a.offset ?? 0, b.offset ?? 0) &&
    (a.playbackStart ?? null) === (b.playbackStart ?? null) &&
    !!a.classicBrowser === !!b.classicBrowser &&
    sameDrumView(a.drumView, b.drumView) &&
    sameStrings(a.warnings, b.warnings) &&
    sameProblems(a.outputProblems, b.outputProblems) &&
    sameStrings(a.recent, b.recent) &&
    sameRecovery(a.recovery, b.recovery)
  );
}

export function samePlayhead(a, b) {
  if (a === b) return true;
  if (!a || !b) return false;
  // While playing, every snapshot re-anchors MapPreview's receivedAt interpolation clock.
  if (a.playing || b.playing) return false;
  return (
    Object.is(a.position, b.position) &&
    (a.musicalPosition ?? null) === (b.musicalPosition ?? null) &&
    Object.is(a.duration, b.duration) &&
    Object.is(a.speed, b.speed)
  );
}

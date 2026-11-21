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
    (a.outputDevice || "") === (b.outputDevice || "") &&
    !!a.songMuted === !!b.songMuted &&
    !!a.songSolo === !!b.songSolo &&
    (a.status || "") === (b.status || "") &&
    !!a.showHome === !!b.showHome &&
    !!a.hasDocument === !!b.hasDocument &&
    !!a.previewWindow === !!b.previewWindow &&
    !!a.objectLines === !!b.objectLines &&
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

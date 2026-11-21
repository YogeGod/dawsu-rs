// Pure comparison helpers to isolate ~85 ms playhead/time updates from unrelated Svelte parts.

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
    !!a.songSolo === !!b.songSolo
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

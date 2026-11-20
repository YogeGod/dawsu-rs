// The top bar's transport and readout (v3 §2.4), as plain functions of the editor state.

const pad = (value, width = 2) => String(value).padStart(width, "0");
const parts = ms => {
  const whole = Math.floor(Math.max(0, ms));
  return [pad(Math.floor(whole / 60000)), pad(Math.floor(whole / 1000) % 60), pad(whole % 1000, 3)];
};

// The playhead time as app.js's clock() writes it: 01:46.052.
const clock = ms => {
  const [minutes, seconds, millis] = parts(ms);
  return minutes + ":" + seconds + "." + millis;
};

// osu!'s editor timestamp, as lazer copies it with nothing selected: 05:24:324 - . Combo numbers
// would go before the dash; the editor state has none yet.
export const osuTimestamp = ms => parts(ms).join(":") + " -";

// The backend sends "bar.beat.tick · 190.0 BPM · 4/4 · snap" and no separate fields.
export function readout(view, scene) {
  const [, tempo, meter] = (scene && view?.musicalPosition?.split(" · ")) || [];
  return {
    time: scene && Number.isFinite(view?.position) ? clock(view.position) : "00:00.000",
    bpm: tempo?.replace(/ BPM$/, "") ?? "–",
    meter: meter ?? "–",
  };
}

export const selectionValue = scene => Number.isFinite(scene?.selection?.start) ? osuTimestamp(scene.selection.start) : null;

// Play, Stop and Cycle need a map; Go to start and Go to end also wait while a job runs, as before.
export function segmentsDisabled(view, scene) {
  const playback = !scene || !!view?.fault;
  return { playback, jump: playback || !!view?.busy };
}

export const transportActions = view => ({
  start: { kind: "start" },
  // Seek clamps to the song's length, so the duration lands on the last moment of the song.
  end: { kind: "seek", ms: view?.duration ?? 0 },
  cycle: { kind: "cycle", on: !view?.cycle },
  stop: { kind: "stop" },
  play: { kind: "play" },
});

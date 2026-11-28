// The top bar's transport and readout (v3 §2.4), as plain functions of the editor state.

const pad = (value, width = 2) => String(value).padStart(width, "0");
const parts = ms => {
  const whole = Math.floor(Math.max(0, ms));
  return [pad(Math.floor(whole / 60000)), pad(Math.floor(whole / 1000) % 60), pad(whole % 1000, 3)];
};

// The playhead time as app.js's clock() writes it: 01:46.052.
export const clock = ms => {
  const [minutes, seconds, millis] = parts(ms);
  return minutes + ":" + seconds + "." + millis;
};

// osu!'s editor timestamp, as lazer copies it with nothing selected: 05:24:324 - . Combo numbers
// would go before the dash; the editor state has none yet.
export const osuTimestamp = ms => parts(ms).join(":") + " -";

// A time typed into the readout, in ms: "01:54.385" (the readout), "01:54:385 (1,2) -" (osu!'s
// timestamp, read up to the first space as osu! does), "1:54" or plain seconds ("114.5"). null if unreadable.
export function parseTime(text) {
  const head = String(text ?? "").trim().split(/\s/)[0];
  let match = /^(\d+):(\d{1,2})(?:[.:](\d{1,3}))?$/.exec(head);
  if (match) {
    const [, minutes, seconds, millis = "0"] = match;
    if (Number(seconds) > 59) return null;
    return Number(minutes) * 60000 + Number(seconds) * 1000 + Number(millis.padEnd(3, "0"));
  }
  match = /^\d+(?:\.\d+)?$/.exec(head);
  return match ? Math.round(Number(head) * 1000) : null;
}

// The backend sends "bar.beat.tick · 190.0 BPM · 4/4 · snap" and no separate fields.
export function readout(view, scene) {
  const [, textTempo, textMeter] = (scene && view?.musicalPosition?.split(" · ")) || [];
  const bpm = scene && Number.isFinite(view?.bpmCenti) && view.bpmCenti > 0
    ? (view.bpmCenti / 100).toFixed(1)
    : (textTempo?.replace(/ BPM$/, "") ?? "–");
  const meter = scene && Number.isFinite(view?.meterNum) && view.meterNum > 0 && Number.isFinite(view?.meterDen) && view.meterDen > 0
    ? `${view.meterNum}/${view.meterDen}`
    : (textMeter ?? "–");
  return {
    time: scene && Number.isFinite(view?.position) ? clock(view.position) : "00:00.000",
    bpm,
    meter,
  };
}

export const selectionValue = scene => Number.isFinite(scene?.selection?.start) ? osuTimestamp(scene.selection.start) : null;

// Play, Stop and Cycle need a map; Go to start and Go to end also wait while a job runs, as before.
// Go to end also needs the song's length.
export function segmentsDisabled(view, scene) {
  const playback = !scene || !!view?.fault;
  const jump = playback || !!view?.busy;
  return { playback, jump, end: jump || !Number.isFinite(view?.duration) };
}

// Play and Cycle toggle, so a second click before the editor's next view would undo the first.
// The gate lets one click through per view.
export function clickGate() {
  let last;
  return view => {
    if (view === last) return false;
    last = view;
    return true;
  };
}

// osu! reads a pasted timestamp up to the space after the dash.
export async function copyText(value, clipboard) {
  try {
    await clipboard.writeText(value + " ");
    return "copied";
  } catch {
    return "failed";
  }
}

export const transportActions = view => ({
  start: { kind: "start" },
  // Seek clamps to the song's length, so the duration lands on the last moment of the song.
  end: { kind: "seek", ms: view?.duration },
  cycle: { kind: "cycle", on: !view?.cycle },
  stop: { kind: "stop" },
  play: { kind: "play" },
});

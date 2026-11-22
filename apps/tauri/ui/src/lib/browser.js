// The Browser parts' plain helpers. app.js builds the model (renderSampleBrowser) and hands it over.

// What the parts show before the first hand-over: nothing to pick, every control greyed out.
export const emptyBrowser = Object.freeze({
  source: "mapset", classic: false, filter: "", busy: true, root: null, rootTitle: null, chooseLibrary: false,
  folder: null, rows: [], preview: null, canAssign: false, canPick: false, target: null,
});

// A key on a sample row. ArrowUp and ArrowDown move to the next sample row and play it; Enter uses
// the sample in the active lane (picking it first). preventDefault keeps the keys from the
// timeline's shortcuts.
export function sampleKey(event, row, send) {
  if (event.ctrlKey || event.altKey || event.metaKey || event.shiftKey) return;
  if (event.key === "Enter") {
    event.preventDefault();
    if (!row.selected) send("pick", row.key);
    send("assign");
    return;
  }
  if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
  event.preventDefault();
  const samples = [...(event.currentTarget.parentElement?.querySelectorAll(".browser-sample:not(:disabled)") ?? [])];
  const next = samples[samples.indexOf(event.currentTarget) + (event.key === "ArrowDown" ? 1 : -1)];
  if (!next) return;
  next.focus();
  send("pick", next.dataset.sample);
}

// v3's "Sample preview · 0.21 s".
export const previewLength = ms => (ms / 1000).toFixed(2) + " s";

// One bar per peak in a viewBox of `peaks.length` by 1, centred on the middle line.
export function outlineBars(peaks) {
  return (peaks ?? []).map((peak, index) => {
    const height = Math.max(0.04, Math.min(1, peak));
    return { x: index + 0.15, y: Math.round((1 - height) / 2 * 1000) / 1000, width: 0.7, height };
  });
}

// The Browser parts' plain helpers. app.js builds the model (renderSampleBrowser) and hands it over.

// What the parts show before the first hand-over: nothing to pick, every control greyed out.
export const emptyBrowser = Object.freeze({
  source: "mapset", classic: false, filter: "", busy: true, root: null, rootTitle: null, defaults: false,
  folder: null, rows: [], preview: null, canAssign: false, canPick: false,
});

// v3's "Sample preview · 0.21 s".
export const previewLength = ms => (ms / 1000).toFixed(2) + " s";

// One bar per peak in a viewBox of `peaks.length` by 1, centred on the middle line.
export function outlineBars(peaks) {
  return (peaks ?? []).map((peak, index) => {
    const height = Math.max(0.04, Math.min(1, peak));
    return { x: index + 0.15, y: Math.round((1 - height) / 2 * 1000) / 1000, width: 0.7, height };
  });
}

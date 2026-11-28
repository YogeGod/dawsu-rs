// The lane and folder headers' fixed parts. app.js hands over the rows (currentLaneHeaders).

// 16 px line icons for the lane's icon tile, by sound. Fixed markup, never lane text.
export const laneIcons = {
  normal: '<circle cx="8" cy="8" r="5.5"/><circle cx="8" cy="8" r="1.4" fill="currentColor"/>',
  whistle: '<path d="M2.5 6H10a2 2 0 1 0-2-2M2.5 10H12a2 2 0 1 1-2 2M2.5 8h4"/>',
  finish: '<ellipse cx="8" cy="9.5" rx="6" ry="2.2"/><path d="M8 7.3V3"/>',
  clap: '<path d="M5.5 9V4a1 1 0 0 1 2 0v4M7.5 7.5V3a1 1 0 0 1 2 0v4.5M9.5 7V4.5a1 1 0 0 1 2 0V10a4 4 0 0 1-4 4h-.5a3.5 3.5 0 0 1-2.8-1.4L2.5 10.3a1 1 0 0 1 1.6-1.2L5.5 11"/>',
  slide: '<path d="M2 9.5c2-3.5 4-3.5 6 0s4 3.5 6 0"/>',
  tick: '<circle cx="8" cy="8" r="2.2" fill="currentColor"/>',
  file: '<path d="M4 2.5h5l3 3v8H4z"/><path d="M6.5 11V9M8.5 11.5V8M10.5 11V9.5"/>',
};

const lanes = count => count + (count === 1 ? " lane" : " lanes");

// v3 8b: "4", or "2/4" while folded; the title says it in full. app.js shows no folder without lanes.
export function folderChip({ count, used, collapsed }) {
  return collapsed
    ? { text: used + "/" + count, title: lanes(count) + ", " + used + " used" }
    : { text: String(count), title: lanes(count) };
}

// "Whistle 2" → ["Whistle", " 2"]: a layer's number is drawn apart so the ellipsis never hides it.
export function splitLayerNumber(name) {
  const match = /^(.*\S)(\s+\d+)$/.exec(name ?? "");
  return match ? [match[1], match[2].replace(/^\s+/, " ")] : [name ?? "", ""];
}

// Folders move by dragging their header, as before.
export const folderDragType = "application/x-dawsu-sample-set";

"use strict";

// Drum hit detection, first slice (Map > Drum hit detection…): a lane prompt, progress, then the
// counts and Apply. A stand-in until the drum detection window; it uses app.js's modal helpers.

let drumDetectMap = null;
let drumDetectRendered = "";

function drumLaneSelect(lanes, chosen) {
  const select = document.createElement("select");
  select.append(new Option("None", ""), ...lanes.map(lane => new Option(lane.name, lane.key)));
  select.value = chosen ?? "";
  return select;
}

function openDrumDetection() {
  const info = view.drumDetect;
  if (!scene || view.busy || drag || !info) return;
  drumDetectMap = scene.path;
  drumDetectRendered = "";
  if (info.state === "idle") showDrumLanes(info);
  else refreshDrumDetection();
}

function closeDrumDetection(cancel = true) {
  const state = view.drumDetect?.state;
  drumDetectMap = null;
  drumDetectRendered = "";
  if (modalKind.startsWith("drum-")) closeModal();
  if (cancel && (state === "running" || state === "ready")) act({ kind: "drum_detect_cancel" });
}

function showDrumModal(kind, content, buttons) {
  modalKind = "";
  modal(kind, "Drum hit detection", content, buttons, () => closeDrumDetection());
}

function showDrumLanes(info, message) {
  const map = drumDetectMap;
  const kick = drumLaneSelect(info.lanes, info.kick);
  const snare = drumLaneSelect(info.lanes, info.snare);
  const content = [labeled("Kick lane", kick), labeled("Snare lane", snare)];
  if (message) content.unshift(paragraph(message));
  drumDetectRendered = "lanes";
  showDrumModal("drum-lanes", content, [
    ["Cancel", () => closeDrumDetection()],
    ["Detect", () => {
      if (!kick.value && !snare.value) {
        showDrumLanes({ ...info, kick: kick.value, snare: snare.value }, "Pick kick and snare lanes first.");
        return;
      }
      drumDetectRendered = "starting";
      act({ kind: "drum_detect_start", map, kick: kick.value || null, snare: snare.value || null });
    }, true],
  ]);
}

function drumCountLines(info) {
  const c = info.counts;
  const lines = [
    "Found " + info.kicks + " kick and " + info.snares + " snare hits.",
    "Adds " + c.added + ": " + c.onObjects + " on objects, " + c.loose + " on grid lines.",
  ];
  if (c.flagged) lines.push(c.flagged + " too far from the grid are left out.");
  if (c.duplicates) lines.push(c.duplicates + " already there.");
  return lines;
}

function refreshDrumDetection() {
  if (!drumDetectMap) return;
  const info = view.drumDetect;
  if (!info || scene?.path !== drumDetectMap) {
    closeDrumDetection(false);
    return;
  }
  if (view.error && drumDetectRendered === "starting") {
    // The start was refused; the editor's message replaces this dialog.
    drumDetectMap = null;
    drumDetectRendered = "";
    return;
  }
  if (info.state === "running") {
    const text = "Detecting drum hits… " + Math.floor(info.done * 100 / Math.max(1, info.total)) + "%";
    if (drumDetectRendered !== text) {
      drumDetectRendered = text;
      showDrumModal("drum-running", [paragraph(text)], [["Cancel", () => closeDrumDetection()]]);
    }
  } else if (info.state === "ready") {
    const key = "ready " + info.review;
    if (drumDetectRendered !== key) {
      drumDetectRendered = key;
      const map = drumDetectMap;
      showDrumModal("drum-ready", drumCountLines(info).map(paragraph), [
        ["Cancel", () => closeDrumDetection()],
        ["Apply", () => {
          closeDrumDetection(false);
          act({ kind: "drum_apply", map, review: info.review });
        }, true],
      ]);
    }
  } else if (drumDetectRendered !== "lanes" && drumDetectRendered !== "starting" && !requests) {
    // The run stopped without a result (cancelled, or the song changed).
    closeDrumDetection(false);
  }
}

setInterval(() => {
  if (!drumDetectMap) return;
  if (view.drumDetect?.state === "running" && !requests && !refreshing) run(() => update(null, false));
  refreshDrumDetection();
}, 100);

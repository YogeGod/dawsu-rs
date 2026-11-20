import { laneGrey, tintName } from "./colour.js";

// What the Inspector shows before app.js hands over its first model (inspectorModel()): everything waits.
export const emptyInspector = Object.freeze({
  busy: false, doc: false, selection: null, scope: "all", hint: "", range: false,
  insertVolume: 100, protect: false, audition: false, hs: false, lane: null,
  bookmarks: [], bookmark: null, history: [], historyPosition: 0,
});

// v3's bookmark rows: dot, label and time. A bookmark with no label is named by its place in the list.
export function bookmarkRows(marks, picked) {
  return (marks ?? []).map((mark, index) => ({
    time: mark.time, label: mark.label || "Bookmark " + (index + 1), clock: mark.clock, current: mark.time === picked,
  }));
}

// History's steps, newest first. Steps after the current one are undone and can be redone.
export function historyRows(steps, position) {
  return steps.map((label, index) => ({ label, position: index, current: index === position, undone: index > position }))
    .reverse();
}

// The Lane colour row: grey and "None" until the mapper picks a colour (Yoge, ADR 0068).
export function laneTint(lane) {
  if (!lane?.colour) return { swatch: laneGrey, name: "None" };
  return { swatch: lane.colour.toLowerCase(), name: tintName(lane.colour) };
}

// A field being typed in shows its held text; otherwise the editor's value (the old setValue rule).
export const shownValue = (held, value) => held ?? value;

// On blur a field goes back to the editor's value, unless it just sent its own: then the editor's
// next update replaces it.
export function fieldHold() {
  let sent = false;
  return {
    sent() {
      sent = true;
    },
    release(field, value) {
      if (!sent) field.value = String(value ?? "");
      sent = false;
    },
  };
}

// The slider body fields, seeded from the selected slider body (re-seeded only when its key changes).
export function sliderBodyFields(body) {
  return {
    set: body?.set ?? "", whistle: !!body?.whistle, whistleSet: body?.whistleSet ?? "auto", volume: body?.volume ?? 100,
  };
}

// Every update sets a switch to the editor's value, as the old show() did, so a refused change flips back.
export function syncChecked(box, value) {
  box.checked = !!value;
}

// Change volume: Set all starts at 100, Relative at 0, as before.
export const startingAmount = mode => (mode === "relative" ? 0 : 100);
export const volumeLabel = mode => (mode === "relative" ? "Change by" : "Set to");

// The Inspector's event handlers. Each sends what app.js's inspectorAction() runs; send comes in
// so tests can drive them with fake events.
export function inspectorHandlers(send) {
  return {
    // hold keeps the sent text in the field when it lets go (blur runs before this returns).
    moveStart(event, field, hold) {
      event.preventDefault();
      const moved = !!send("moveStart", field.value);
      if (moved) {
        hold?.sent();
        field.blur();
      }
      return moved;
    },
    volume(event, mode, amount) {
      event.preventDefault();
      send("volume", typeof amount === "number" ? amount : NaN, mode === "relative");
    },
    nudge: amount => send("nudge", amount),
    scope: value => send("scope", value),
    command: id => send("command", id),
    sliderBody(event, fields) {
      event.preventDefault();
      send("sliderBody", fields);
    },
    insert(event, hold) {
      send("insert", event.currentTarget.valueAsNumber);
      hold?.sent();
    },
    toggle: (kind, event) => send("toggle", kind, event.currentTarget.checked),
    open: kind => send(kind),
    // A bookmark row picks its bookmark and goes to it; add, goTo, rename and remove act on the pick.
    pickBookmark: time => send("pickBookmark", time),
    bookmark: kind => send("bookmark", kind),
    history: position => send("history", position),
  };
}

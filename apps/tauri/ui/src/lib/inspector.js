import { laneGrey, tintName } from "./colour.js";
import { clock } from "./transport.js";

// What the Inspector shows before app.js hands over its first model (inspectorModel()): everything waits.
export const emptyInspector = Object.freeze({
  busy: false, doc: false, selection: null, scope: "all", hint: "", range: false,
  insertVolume: 100, protect: false, audition: false, hs: false, lane: null,
  bookmarks: [], bookmark: null, history: [], historyPosition: 0, problemStrip: null,
});

// v3's bookmark rows: dot, label and time. A bookmark with no label shows its time alone, as the old list did.
export function bookmarkRows(marks, picked) {
  return (marks ?? []).map(mark => ({
    time: mark.time, label: mark.label || "", clock: mark.clock, current: mark.time === picked,
  }));
}

// A bookmark row gets a tooltip with its label only while the label is cut off; set on hover, so
// it follows the dock's width.
export function cutOffTitle(row, label) {
  const shown = row.querySelector(".insp-mark-label");
  if (label && shown && shown.scrollWidth > shown.clientWidth) row.title = label;
  else row.removeAttribute("title");
}

// Scrolls list just enough to show item whole, as scrollIntoView's block "nearest" does, but
// only the list: scrollIntoView would scroll the Inspector too. A folded list has no box and stays.
export function scrollNearest(list, item) {
  if (!item || !list.getClientRects().length) return;
  const box = list.getBoundingClientRect();
  const row = item.getBoundingClientRect();
  if (row.top < box.top) list.scrollTop -= box.top - row.top;
  else if (row.bottom > box.bottom) list.scrollTop += row.bottom - box.bottom;
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

// A time as Next problem writes it: 01:46.052, with a minus sign before the song starts.
export const signedClock = ms => (ms < 0 ? "−" : "") + clock(Math.abs(ms));

const plural = (count, one, many) => count === 1 ? one : count + " " + many;

// Inspector › Difficulty's problem strip (v3 6a), from the editor's own data only (inspectorModel()'s
// problemStrip): a mark for each time this difficulty has a Save problem (a reason Save can't write
// a moment; Generate's problems belong to the generated file and stay out) and for each hit with no
// object, ticks for the objects, and a frame for the part the timeline shows. Marks of one kind in
// one pixel column of the strip merge, so hundreds of hits stay usable.
export function problemStrip(data, width = 200) {
  if (!data) return { name: "", marks: [], ticks: [], frame: null, unsaved: 0, orphaned: 0 };
  const columns = Math.max(1, Math.round(width));
  const length = data.duration > 0 ? data.duration : 1;
  const fraction = time => Math.min(Math.max(time / length, 0), 1);
  const percent = value => Math.round(value * 1e6) / 1e4 + "%";
  const column = time => Math.floor(fraction(time) * columns);
  const messages = new Map();
  for (const { target, time, message } of data.problems ?? []) {
    if (target !== data.path) continue;
    if (!messages.has(time)) messages.set(time, []);
    if (!messages.get(time).includes(message)) messages.get(time).push(message);
  }
  const single = [
    ...[...messages].map(([time, list]) => {
      const text = list.join(" ");
      return { kind: "unsaved", time, label: /^At \S+ ms\b/.test(text) ? text : signedClock(time) + " · " + text };
    }),
    ...(data.orphaned ?? []).map(time => ({ kind: "orphaned", time, label: signedClock(time) + " · Hit with no object" })),
  ].sort((first, second) => first.time - second.time);
  const marks = [];
  const last = {};
  for (const mark of single) {
    const into = last[mark.kind];
    if (into && column(into.time) === column(mark.time)) {
      into.count++;
      into.label = signedClock(into.time) + " · " + (into.kind === "orphaned"
        ? into.count + " hits with no object" : into.count + " moments Save can't write");
      continue;
    }
    last[mark.kind] = { ...mark, count: 1, key: mark.kind + " " + mark.time, left: percent(fraction(mark.time)) };
    marks.push(last[mark.kind]);
  }
  const ticks = [...new Set((data.objects ?? []).map(column))].map(index => percent(index / columns));
  const frame = Number.isFinite(data.viewStart) && data.viewWidth > 0
    ? { left: percent(fraction(data.viewStart)), width: percent(fraction(data.viewStart + data.viewWidth) - fraction(data.viewStart)) }
    : null;
  return { name: data.name ?? "", marks, ticks, frame, unsaved: messages.size, orphaned: (data.orphaned ?? []).length };
}

// The strip's marks: a click goes to the mark's time; the strip is one Tab stop, and Left, Right,
// Home and End move between its marks. key() returns the mark to focus, or null for other keys.
export function problemMarkHandlers(send) {
  const moves = { ArrowLeft: (index) => index - 1, ArrowRight: (index) => index + 1, Home: () => 0, End: (_, count) => count - 1 };
  return {
    click: mark => send("jump", mark.time),
    key(event, index, count) {
      const move = moves[event.key];
      if (!move || !count) return null;
      event.preventDefault();
      return Math.min(Math.max(move(index, count), 0), count - 1);
    },
  };
}

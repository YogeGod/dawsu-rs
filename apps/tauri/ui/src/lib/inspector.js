import { laneGrey, tintName } from "./colour.js";

// The Lane tint row: grey and "None" until the mapper picks a colour (Yoge, ADR 0068).
export function laneTint(lane) {
  if (!lane?.colour) return { swatch: laneGrey, name: "None" };
  return { swatch: lane.colour.toLowerCase(), name: tintName(lane.colour) };
}

// The Inspector's event handlers. Each sends what app.js's inspectorAction() runs; send comes in
// so tests can drive them with fake events.
export function inspectorHandlers(send) {
  return {
    moveStart(event, field) {
      event.preventDefault();
      if (send("moveStart", field.value)) field.blur();
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
    insert: event => send("insert", event.currentTarget.valueAsNumber),
    toggle: (kind, event) => send("toggle", kind, event.currentTarget.checked),
    open: kind => send(kind),
  };
}

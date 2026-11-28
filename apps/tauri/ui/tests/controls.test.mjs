import assert from "node:assert/strict";
import { test } from "node:test";
import { presetSummary, toolTitle, WHEEL_HINTS, wheelIntent } from "../src/lib/controls.js";
import { hintsFor } from "../src/lib/hints.js";

// The timeline wheel handler picks the first true intent in this order (timeline.js).
const ORDER = ["rows", "snap", "zoom", "pan", "seek"];
const winner = flags => ORDER.find(name => flags[name]) ?? null;

const COMBOS = [
  [[], "none"], [["ctrl"], "ctrl"], [["shift"], "shift"], [["alt"], "alt"],
  [["ctrl", "shift"], "ctrl+shift"], [["ctrl", "alt"], "ctrl+alt"], [["shift", "alt"], "shift+alt"],
  [["ctrl", "shift", "alt"], "ctrl+shift+alt"],
];
const mods = list => ({ ctrl: list.includes("ctrl"), shift: list.includes("shift"), alt: list.includes("alt") });

// Written out by hand from the editor's behaviour: vertical wheel over the grid.
const EXPECTED = {
  DAW: {
    none: null, ctrl: "zoom", shift: "pan", alt: "snap",
    "ctrl+shift": "rows", "ctrl+alt": "zoom", "shift+alt": "snap", "ctrl+shift+alt": "rows",
  },
  "osu!": {
    none: "seek", ctrl: "snap", shift: "seek", alt: "zoom",
    "ctrl+shift": "rows", "ctrl+alt": "snap", "shift+alt": "zoom", "ctrl+shift+alt": "rows",
  },
};

test("wheelIntent gives the gesture for every modifier in both presets", () => {
  for (const [preset, table] of Object.entries(EXPECTED)) {
    for (const [list, label] of COMBOS) {
      const flags = wheelIntent({ osu: preset === "osu!", ...mods(list), horizontal: false, overGrid: true });
      assert.equal(winner(flags), table[label], `${preset} ${label}`);
    }
  }
});

test("wheelIntent: off the grid a plain osu! wheel does nothing, Shift still seeks; a sideways wheel pans", () => {
  const off = { horizontal: false, overGrid: false };
  assert.equal(winner(wheelIntent({ osu: true, ...mods([]), ...off })), null);
  assert.equal(winner(wheelIntent({ osu: true, ...mods(["shift"]), ...off })), "seek");
  assert.equal(winner(wheelIntent({ osu: false, ...mods([]), ...off })), null);
  for (const osu of [false, true]) {
    assert.equal(winner(wheelIntent({ osu, ...mods([]), horizontal: true, overGrid: true })), "pan", `osu=${osu}`);
    assert.equal(winner(wheelIntent({ osu, ...mods(["ctrl"]), horizontal: true, overGrid: true })), "pan");
  }
});

test("every wheel hint names keys that do what the hint says, and every gesture has a hint", () => {
  for (const preset of ["DAW", "osu!"]) {
    const hints = WHEEL_HINTS[preset];
    for (const [intent, text] of Object.entries(hints)) {
      if (text == null) continue;
      assert.match(text, /^(?:(?:Ctrl|Shift|Alt)\+)*Wheel$/, `${preset} ${intent}`);
      const list = text.split("+").slice(0, -1).map(part => part.toLowerCase());
      const flags = wheelIntent({ osu: preset === "osu!", ...mods(list), horizontal: false, overGrid: true });
      assert.equal(winner(flags), intent, `${preset} "${text}"`);
    }
    // A gesture reachable with a vertical wheel must have a hint; one without a hint must be unreachable.
    const reachable = new Set(Object.values(EXPECTED[preset]).filter(Boolean));
    for (const intent of ORDER) assert.equal(hints[intent] != null, reachable.has(intent), `${preset} ${intent}`);
  }
  assert.equal(WHEEL_HINTS.DAW.zoom, "Ctrl+Wheel");
  assert.equal(WHEEL_HINTS["osu!"].zoom, "Alt+Wheel");
});

test("toolTitle names the tool and its key, or just the tool when it has none", () => {
  assert.equal(toolTitle("Draw", "3"), "Draw (3)");
  assert.equal(toolTitle("Erase", "Shift+E"), "Erase (Shift+E)");
  assert.equal(toolTitle("Range", null), "Range");
});

test("presetSummary names each preset's tool keys, zoom and wheel gestures", () => {
  const osu = presetSummary("osu!");
  assert.match(osu, /^osu! Editor: /);
  assert.match(osu, /1 Select, 2 Draw, 4 Erase, 5 Mute, 6 Ramp/);
  assert.match(osu, /Alt\+Wheel zooms/);
  assert.match(osu, /Ctrl\+Wheel steps the snap/);
  assert.match(osu, /Wheel seeks/);
  assert.doesNotMatch(osu, /\d\s*[–-]\s*\d/, "no key range such as 1–3");
  assert.doesNotMatch(osu, /Grab/);

  const daw = presetSummary("DAW");
  assert.match(daw, /^DAW-like: /);
  assert.match(daw, /1 Select, 2 Grab, 3 Draw, 4 Erase, 5 Mute, 6 Ramp/);
  assert.match(daw, /Ctrl\+Wheel zooms/);
  assert.match(daw, /Alt\+Wheel steps the snap/);
  assert.match(daw, /Shift\+Wheel pans/);
  assert.doesNotMatch(daw, /seeks/);
});

test("timeline hints take their wheel keys from WHEEL_HINTS", () => {
  for (const preset of ["DAW", "osu!"]) {
    for (const tool of ["Select", "Draw", "Erase", "Mute", "Grab", "Range", "Ramp"]) {
      for (const row of hintsFor("timeline", tool, preset)) {
        if (!row.keys.includes("Wheel")) continue;
        const intent = { Zoom: "zoom", Seek: "seek", Pan: "pan", Snap: "snap" }[row.action];
        assert.ok(intent, `${preset} ${tool} ${row.action}`);
        assert.equal(row.keys, WHEEL_HINTS[preset][intent], `${preset} ${tool} ${row.action}`);
      }
    }
  }
});

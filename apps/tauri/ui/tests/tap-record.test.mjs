import assert from "node:assert/strict";
import { test } from "node:test";
import {
  HOME_ROW_TAP_CODES,
  NUMPAD_TAP_CODES,
  osuReloadToast,
  resolveVisibleTapRow,
  tapAwareRecordToast,
  tapLaneSlotForKey,
} from "../src/lib/tapRecord.js";

test("Numpad 1–9 always map to visible lanes 0–8; A–L map only when tap recording is armed", () => {
  assert.equal(NUMPAD_TAP_CODES.length, 9);
  assert.equal(HOME_ROW_TAP_CODES.length, 9);
  assert.equal(tapLaneSlotForKey({ code: "Numpad1" }, false), 0);
  assert.equal(tapLaneSlotForKey({ code: "Numpad9" }, false), 8);
  assert.equal(tapLaneSlotForKey({ code: "Numpad1", ctrl: true }, false), null);

  assert.equal(tapLaneSlotForKey({ code: "KeyA" }, false), null);
  assert.equal(tapLaneSlotForKey({ code: "KeyA" }, true), 0);
  assert.equal(tapLaneSlotForKey({ code: "KeyL" }, true), 8);
  assert.equal(tapLaneSlotForKey({ code: "KeyA", shift: true }, true), null);
  assert.equal(tapLaneSlotForKey({ code: "Space" }, true), null);
});

test("resolveVisibleTapRow skips non-integer folder rows and bounds-checks", () => {
  const laneRows = ["normal-header", 0, 2, "soft-header", 5];
  assert.equal(resolveVisibleTapRow(laneRows, 0), 0);
  assert.equal(resolveVisibleTapRow(laneRows, 1), 2);
  assert.equal(resolveVisibleTapRow(laneRows, 2), 5);
  assert.equal(resolveVisibleTapRow(laneRows, 3), null);
  assert.equal(resolveVisibleTapRow([], 0), null);
});

test("tapAwareRecordToast explains keyboard tap recording when no MIDI port is attached", () => {
  const armed = tapAwareRecordToast({ armed: true, portName: null, bufferedNotes: 0, committedNotes: 0 });
  assert.equal(armed.title, "Tap recording armed");
  assert.match(armed.body, /A–L/);
  assert.match(armed.body, /Numpad 1–9/);

  const withPort = tapAwareRecordToast({ armed: true, portName: "TD-17" });
  assert.equal(withPort.title, "MIDI recording armed");
  assert.match(withPort.body, /TD-17/);

  const committed = tapAwareRecordToast({ armed: false, portName: null, committedNotes: 4 });
  assert.equal(committed.tone, "success");
  assert.match(committed.body, /4 hits/);
});

test("osuReloadToast shapes stable, lazer external edit, and fallback outcomes cleanly", () => {
  assert.equal(osuReloadToast({ status: "reloaded_stable", message: "Sent Ctrl+L" }).tone, "success");
  assert.equal(osuReloadToast({ status: "lazer_external_edit", message: "Click Finish" }).title, "osu!lazer external edit");
  assert.equal(osuReloadToast({ status: "no_matching_window", message: "Not open" }).tone, "info");
  assert.equal(osuReloadToast(null), null);
});

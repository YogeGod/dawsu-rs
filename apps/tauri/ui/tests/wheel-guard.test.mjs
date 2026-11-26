import assert from "node:assert/strict";
import { test } from "node:test";
import { altWheelDbStep, attachAltWheelFader, handleInputWheelGuard } from "../src/lib/wheelGuard.js";
import { faderStops, stepGain } from "../src/lib/mixer.js";

function makeInput(type, { allow = false } = {}) {
  let blurred = false;
  return {
    type,
    blurred: () => blurred,
    blur() { blurred = true; },
    closest(selector) {
      if (selector === "[data-wheel-allow]") return allow ? {} : null;
      if (selector.includes(type)) return this;
      return null;
    },
  };
}

test("bare wheel over focused input[type=number] or [type=range] blurs it without calling preventDefault", () => {
  const num = makeInput("number");
  let prevented = false;
  const doc = { activeElement: num };
  const res = handleInputWheelGuard({
    target: num,
    deltaY: 120,
    preventDefault() { prevented = true; },
  }, doc);

  assert.deepEqual(res, { guarded: true, blurred: true, kind: "number" });
  assert.equal(num.blurred(), true);
  assert.equal(prevented, false, "must keep native container smooth/inertial scroll");

  const range = makeInput("range");
  const unfocusedRes = handleInputWheelGuard({
    target: range,
    deltaY: 120,
    preventDefault() { prevented = true; },
  }, doc);
  assert.deepEqual(unfocusedRes, { guarded: true, blurred: false, kind: "range" });
  assert.equal(prevented, false);

  const allowed = makeInput("number", { allow: true });
  assert.equal(handleInputWheelGuard({ target: allowed, deltaY: 120 }, { activeElement: allowed }).guarded, false);
});

test("Fader ignores bare wheel without preventDefault and handles Alt+Wheel in 1 dB (0.5 dB Shift) steps", () => {
  const listeners = new Map();
  const node = {
    addEventListener(name, fn) { listeners.set(name, fn); },
    removeEventListener(name) { listeners.delete(name); },
  };
  let shown = 0.5;
  const sent = [];
  const cleanup = attachAltWheelFader(() => ({ shown, min: 0, max: 2, disabled: false, send: (v) => { shown = v; sent.push(v); } }))(node);
  const onWheel = listeners.get("wheel");
  assert.equal(typeof onWheel, "function");

  let prevented = 0;
  let stopped = 0;
  onWheel({ altKey: false, deltaY: -120, preventDefault() { prevented++; }, stopPropagation() { stopped++; } });
  assert.equal(prevented, 0);
  assert.equal(stopped, 0);
  assert.equal(sent.length, 0);

  const expectedUp = stepGain(0.5, 1, 0, 2);
  onWheel({ altKey: true, deltaY: -120, preventDefault() { prevented++; }, stopPropagation() { stopped++; } });
  assert.equal(prevented, 1);
  assert.equal(stopped, 1);
  assert.ok(Math.abs(shown - expectedUp) < 1e-9);

  const beforeShift = shown;
  const expectedFineDown = stepGain(beforeShift, -0.5, 0, 2);
  assert.equal(altWheelDbStep({ altKey: true, shiftKey: true, deltaY: 120 }), -0.5);
  onWheel({ altKey: true, shiftKey: true, deltaY: 120, preventDefault() { prevented++; }, stopPropagation() { stopped++; } });
  assert.ok(Math.abs(shown - expectedFineDown) < 1e-9);
  assert.ok(Array.isArray(faderStops));

  cleanup();
  assert.equal(listeners.size, 0);
});

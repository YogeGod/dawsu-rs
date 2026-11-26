// DAW wheel protection: blurs focused numeric/range inputs on bare wheel so Chromium
// never steps values while preserving native smooth/inertial container scrolling, plus
// opt-in Alt+Wheel fine ±1 dB adjustment on Mixer Fader controls.
import { stepGain } from "./mixer.js";

const GUARDED_INPUT_SELECTOR = 'input[type="number"], input[type="range"]';

export function handleInputWheelGuard(event, doc = event?.target?.ownerDocument ?? null) {
  const input = event?.target?.closest?.(GUARDED_INPUT_SELECTOR);
  if (!input || event.ctrlKey || event.metaKey || event.altKey) {
    return { guarded: false, blurred: false, kind: null };
  }
  if (input.closest?.("[data-wheel-allow]")) {
    return { guarded: false, blurred: false, kind: null };
  }
  let blurred = false;
  if (doc?.activeElement === input && typeof input.blur === "function") {
    input.blur();
    blurred = true;
  }
  return { guarded: true, blurred, kind: input.type || null };
}

export function attachGlobalWheelGuard(doc) {
  if (!doc?.addEventListener) return () => {};
  const onWheel = (event) => {
    handleInputWheelGuard(event, doc);
  };
  doc.addEventListener("wheel", onWheel, { capture: true, passive: true });
  return () => doc.removeEventListener("wheel", onWheel, true);
}

export function altWheelDbStep(event) {
  if (!event?.altKey || event.ctrlKey || event.metaKey || !event.deltaY) return 0;
  const dir = event.deltaY < 0 ? 1 : -1;
  return dir * (event.shiftKey ? 0.5 : 1);
}

export function attachAltWheelFader(getOptions) {
  return (node) => {
    if (!node?.addEventListener) return () => {};
    const onWheel = (event) => {
      const db = altWheelDbStep(event);
      if (!db) return;
      const opts = getOptions?.();
      if (!opts || opts.disabled || opts.shown == null) return;
      event.preventDefault?.();
      event.stopPropagation?.();
      const next = stepGain(opts.shown, db, opts.min, opts.max);
      if (next !== opts.shown) opts.send?.(next);
    };
    node.addEventListener("wheel", onWheel, { passive: false });
    return () => node.removeEventListener("wheel", onWheel);
  };
}

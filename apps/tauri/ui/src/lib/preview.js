// The map preview's timing and geometry (step 1: plain shapes). The model comes from Rust's
// map_preview_objects: each object's stacked place, times, path and combo, with the map's circle
// radius, approach time (preempt) and fade-in, as osu!lazer works them out at 131fe72. This file
// only decides what shows at a given time; MapPreview.svelte draws it.

// osu!'s playfield, and the 640 × 480 "screen" it is centred in so objects near the edge still fit
// (OsuPlayfield.cs:47 BASE_SIZE; the old app's PlayfieldView.cs:30-31).
export const PLAYFIELD = { width: 512, height: 384 };
const SCREEN = { width: 640, height: 480 };
// The editor's look after an object's time: it fades out over 700 ms (DrawableHitCircle.cs:334-351,
// DrawableSlider.cs:376-384 at 131fe72).
export const AFTER_HIT = 700;
// A spinner fades 240 ms after its end, as the old app drew it (PlayfieldView.cs:396-435 at c81774e).
export const SPINNER_FADE = 240;
// The approach circle starts at 4× and reaches 0.9 opacity (DrawableHitCircle.cs:190-198).
const APPROACH_START = 4;
const APPROACH_ALPHA = 0.9;

export const clamp01 = value => Math.min(1, Math.max(0, value));

// Where the playfield sits in a box of width × height: the playfield pixel size and the corner.
export function fit(width, height) {
  const scale = Math.max(0, Math.min(width / SCREEN.width, height / SCREEN.height));
  return {
    scale,
    x: (width - PLAYFIELD.width * scale) / 2,
    y: (height - PLAYFIELD.height * scale) / 2,
  };
}

// The time the preview shows: the editor's position, moved on by the clock while playing, so the
// preview runs smoothly between the editor's updates (about every 85 ms). While playing it never
// steps back by less than BACK_STEP (an update that lands a few ms behind the clock), and it stops
// at the song's end.
const BACK_STEP = 50;
export function clock(view, receivedAt, now, previous = null) {
  if (!view) return 0;
  if (!view.playing) return view.position;
  let time = view.position + Math.max(0, now - receivedAt) * (view.speed || 1);
  if (view.duration > 0) time = Math.min(time, view.duration);
  if (previous !== null && time < previous && previous - time < BACK_STEP) time = previous;
  return time;
}

// When an object stops showing.
const fadeEnd = object => object.end + (object.kind === "spinner" ? SPINNER_FADE : AFTER_HIT);

// First index in [lo, hi) where `pred(i)` holds, or `hi` if none.
function bsearch(lo, hi, pred) {
  while (lo < hi) {
    const mid = (lo + hi) >>> 1;
    if (pred(mid)) hi = mid;
    else lo = mid + 1;
  }
  return lo;
}

const maxEndCache = new WeakMap();

// Running maximum of `fadeEnd` across `objects`, so long sliders/spinners that started earlier are never skipped.
function maxEndSoFar(objects) {
  const cached = maxEndCache.get(objects);
  if (cached && cached.length === objects.length) return cached;
  const maxEnds = new Float64Array(objects.length);
  let max = -Infinity;
  for (let i = 0; i < objects.length; i++) {
    const end = fadeEnd(objects[i]);
    if (end > max) max = end;
    maxEnds[i] = max;
  }
  maxEndCache.set(objects, maxEnds);
  return maxEnds;
}

// The objects on screen at `time`, in drawing order: the latest first, so the next one is on top.
export function visible(model, time) {
  const objects = model?.objects;
  if (!objects?.length) return [];
  const maxEnds = maxEndSoFar(objects);
  const start = bsearch(0, objects.length, i => maxEnds[i] >= time);
  const preempt = model.preempt;
  const shown = [];
  for (let i = start; i < objects.length; i++) {
    const object = objects[i];
    const appear = object.time - preempt;
    if (appear > time) break;
    if (time >= appear && time <= fadeEnd(object)) shown.push(object);
  }
  return shown.reverse();
}

// Index range [start, end) of consecutive object pairs `(i, i + 1)` whose follow points can be visible at `time`.
export function followPointRange(objects, time, preempt) {
  const n = objects?.length ?? 0;
  if (n < 2) return { start: 0, end: 0 };
  const minNextTime = time - 150;
  const start = bsearch(1, n, i => objects[i].time >= minNextTime) - 1;
  const maxPrevTime = time + preempt;
  const end = bsearch(start, n - 1, i => objects[i].time > maxPrevTime);
  return { start, end };
}

// An object's opacity: it fades in over the fade-in time from (time − preempt), stays until its end,
// then fades out.
export function opacity(model, object, time) {
  const appear = object.time - model.preempt;
  if (time < object.end) return clamp01((time - appear) / Math.max(1, model.fadeIn));
  const fade = object.kind === "spinner" ? SPINNER_FADE : AFTER_HIT;
  return clamp01(1 - (time - object.end) / fade);
}

// The approach circle around a circle or a slider head, or null once its time has come.
export function approach(model, object, time) {
  if (object.kind === "spinner" || time >= object.time) return null;
  const appear = object.time - model.preempt;
  if (time < appear) return null;
  const progress = clamp01((time - appear) / model.preempt);
  const fadeTime = Math.min(2 * model.fadeIn, model.preempt);
  return {
    scale: APPROACH_START + (1 - APPROACH_START) * progress,
    alpha: APPROACH_ALPHA * clamp01((time - appear) / Math.max(1, fadeTime)),
  };
}

// How much of a slider's path is drawn: it snakes in over the first third of the preempt
// (SnakingSliderBody.cs:80-98, DrawableSlider.cs:336-340).
export function snake(model, object, time) {
  return clamp01((time - (object.time - model.preempt)) / (model.preempt / 3));
}

// The path's length (the last cumulative length).
export const pathLength = object => object.lengths?.[object.lengths.length - 1] ?? 0;

// The point `distance` along a slider's flat path, as SliderPath.interpolateVertices finds it.
export function pointAt(object, distance) {
  const path = object.path ?? [];
  const lengths = object.lengths ?? [];
  const count = path.length / 2;
  if (!count) return { x: object.x, y: object.y };
  let index = bsearch(0, lengths.length, i => lengths[i] >= distance);
  if (index >= lengths.length) index = count;
  if (index <= 0) return { x: path[0], y: path[1] };
  if (index >= count) return { x: path[2 * count - 2], y: path[2 * count - 1] };
  const [d0, d1] = [lengths[index - 1], lengths[index]];
  const [x0, y0, x1, y1] = [path[2 * index - 2], path[2 * index - 1], path[2 * index], path[2 * index + 1]];
  if (Math.abs(d1 - d0) < 1e-3) return { x: x0, y: y0 };
  const w = (distance - d0) / (d1 - d0);
  return { x: x0 + (x1 - x0) * w, y: y0 + (y1 - y0) * w };
}

// The path from its start to `distance`, as points (SliderPath.GetPathToProgress from 0).
export function pathTo(object, distance) {
  const path = object.path ?? [];
  const lengths = object.lengths ?? [];
  const limit = Math.min(Math.ceil(path.length / 2), lengths.length);
  const end = bsearch(0, limit, i => !(lengths[i] <= distance));
  const points = [];
  for (let i = 0; i < end; i++) points.push({ x: path[2 * i], y: path[2 * i + 1] });
  points.push(pointAt(object, distance));
  return points;
}

// The span a slider is on at `time` and how far along the path the ball is (0 at the start, 1 at
// the end), going back and forth once per span (IHasPathWithRepeats.cs: ProgressAt, SpanAt).
export function sliderProgress(object, time) {
  const spans = object.spans ?? 1;
  const whole = clamp01((time - object.time) / Math.max(1e-9, object.end - object.time));
  const span = Math.min(spans - 1, Math.floor(whole * spans));
  let along = (whole * spans) % 1;
  if (whole >= 1) along = 1;
  if ((whole >= 1 ? spans - 1 : span) % 2 === 1) along = 1 - along;
  return { span, along };
}

// The slider ball while the slider runs, else null.
export function ball(object, time) {
  if (object.kind !== "slider" || time < object.time || time > object.end) return null;
  const { along } = sliderProgress(object, time);
  return pointAt(object, along * pathLength(object));
}

// The reverse arrow on the end the ball is heading to, while that end still turns it back.
// `angle` points back along the path, in radians.
export function reverseArrow(object, time) {
  const spans = object.spans ?? 1;
  if (object.kind !== "slider" || spans < 2 || time > object.end) return null;
  const span = time < object.time ? 0 : sliderProgress(object, time).span;
  if (span >= spans - 1) return null;
  const length = pathLength(object);
  const toEnd = span % 2 === 0;
  const at = pointAt(object, toEnd ? length : 0);
  const before = pointAt(object, toEnd ? Math.max(0, length - 4) : Math.min(length, 4));
  return { ...at, angle: Math.atan2(before.y - at.y, before.x - at.x) };
}

// A combo colour from the model's list, with a grey for a missing one.
export const comboColour = (model, object) => model.colours?.[object.colour] ?? "#a0a0a0";

// The ball's place at any time in the slider (clamped to its start and end).
export function ballAt(object, time) {
  const { along } = sliderProgress(object, Math.min(object.end, Math.max(object.time, time)));
  return pointAt(object, along * pathLength(object));
}

// How long a span lasts.
const spanDuration = object => (object.end - object.time) / Math.max(1, object.spans ?? 1);

// A circle piece's opacity: its fade-in before `hitTime`, then 1 → 0 over AFTER_HIT, as lazer's
// editor shows a passed object (DrawableHitCircle.cs:334-351).
export function pieceOpacity(model, object, time, hitTime = object.time) {
  if (time <= hitTime) return clamp01((time - (object.time - model.preempt)) / Math.max(1, model.fadeIn));
  return clamp01(1 - (time - hitTime) / AFTER_HIT);
}

// A slider body's or spinner's opacity: its fade-in, then 1 → 0 over SPINNER_FADE after its end.
export function bodyOpacity(model, object, time) {
  if (time > object.end) return clamp01(1 - (time - object.end) / SPINNER_FADE);
  return clamp01((time - (object.time - model.preempt)) / Math.max(1, model.fadeIn));
}

// The ticks still to come at `time`: each shows from partway through its span (SliderTick.cs:22-31:
// half the time since the span began, plus 200 ms on later spans or 0.66 × preempt on the first),
// popping in over 150 ms and growing from half size over 600 ms. `since` is the time since it showed.
export function ticksShown(model, object, time) {
  const duration = spanDuration(object);
  if (!(duration > 0)) return [];
  const shown = [];
  for (const at of object.ticks ?? []) {
    if (at <= time) continue;
    const span = Math.min((object.spans ?? 1) - 1, Math.max(0, Math.floor((at - object.time) / duration)));
    const spanStart = object.time + span * duration;
    const appears = at - ((at - spanStart) / 2 + (span > 0 ? 200 : model.preempt * 0.66));
    if (time < appears) continue;
    shown.push({ ...ballAt(object, at), since: time - appears });
  }
  return shown;
}

// The reverse arrows still to come at `time`, one per turn: the first shows once the body has snaked
// in (preempt / 3), later ones two spans ahead (DrawableOsuHitObject.cs:155-171, SliderEndCircle.cs:38-43).
// `angle` (radians) points back into the path; `since` is the time since it showed.
export function arrowsShown(model, object, time) {
  const spans = object.spans ?? 1;
  const path = object.path ?? [];
  if (spans < 2 || path.length < 4) return [];
  const duration = spanDuration(object);
  const length = pathLength(object);
  const shown = [];
  for (let node = spans - 1; node >= 1; node--) {
    const nodeTime = object.time + node * duration;
    const appears = node === 1 ? object.time - model.preempt + model.preempt / 3 : nodeTime - duration * 2;
    if (time < appears || time >= nodeTime) continue;
    const atEnd = node % 2 === 1;
    const at = pointAt(object, atEnd ? length : 0);
    const towards = pointAt(object, atEnd ? Math.max(0, length - 4) : Math.min(length, 4));
    // The first fades in over 150 ms, later ones over min(span, 150) (DrawableOsuHitObject.cs ApplyRepeatFadeIn).
    const fade = node === 1 ? 150 : Math.min(duration, 150);
    shown.push({ ...at, angle: Math.atan2(towards.y - at.y, towards.x - at.x), since: time - appears, fade });
  }
  return shown;
}

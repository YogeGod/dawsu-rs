// Dense Canvas 2D renderer for the timeline grid, waveform, gameplay objects, hit diamonds,
// volume lane, and song overview strip. Kept on Canvas for Chromium + WebKit performance.
import {
  bookmarkFlagHeight, gridLines, limit, locatorFlagHeight, locatorFlagWidth,
  objectReach, objectSize, objectWhite, overviewColumns, reducedDivisor,
  rulerBandHeight, slateTimeline, snapColour, snapColours, snapTickHeights,
  trackShade, visibleWarningMarkers, volumeColour,
} from "./helpers.js";
import { visibleViewerOverlay } from "../lib/node-graph.js";

export function attachCanvas(ctx) {
  const { env, state, dom, elements } = ctx;
  const { $ } = dom;
  const perf = () => env.performance ?? performance;
  const raf = (callback) => (env.requestAnimationFrame ?? requestAnimationFrame)(callback);

  // The audio clock reports in steps of its buffer, so a report up to this far behind the drawn line
  // leaves the line where it is instead of pulling it back a pixel or two.
  const playheadHold = 20;
  // With no new view for this long (the editor is busy), the line stops instead of running on.
  const playheadReach = 250;

  function prepare(canvas, height = canvas.clientHeight) {
    const width = canvas.clientWidth;
    const uiScale = (Number(env.document?.documentElement?.dataset?.uiScale) || 100) / 100;
    const scale = (env.window.devicePixelRatio || 1) * uiScale;
    const w = Math.max(1, Math.round(width * scale));
    const h = Math.max(1, Math.round(height * scale));
    if (canvas.width !== w) canvas.width = w;
    if (canvas.height !== h) canvas.height = h;
    const context2d = canvas.getContext("2d");
    context2d.setTransform(scale, 0, 0, scale, 0, 0);
    context2d.clearRect(0, 0, width, height);
    return { ctx: context2d, width, height };
  }

  function line(context2d, color, x1, y1, x2, y2) {
    context2d.strokeStyle = color;
    context2d.beginPath();
    context2d.moveTo(x1 + 0.5, y1);
    context2d.lineTo(x2 + 0.5, y2);
    context2d.stroke();
  }

  // The bar number each timing section starts on, as the editor counts them (tempo.rs bar_beat_at): a
  // part bar before the next red line counts as a whole one. Built once per timing snapshot.
  function sectionBars(sections) {
    if (state.sectionBarCache?.sections !== sections) {
      const bars = [];
      let bar = 1;
      for (const [i, [origin, beat, meter]] of sections.entries()) {
        bars.push(bar);
        const next = sections[i + 1]?.[0];
        if (next !== undefined && beat > 0 && meter) bar += Math.max(0, Math.ceil((next - origin) / (beat * meter) - 1e-9));
      }
      state.sectionBarCache = { sections, bars };
    }
    return state.sectionBarCache.bars;
  }

  function drawTimingGrid(context2d, start, end, zoom, top, bottom, ruler = false) {
    const useSnap = state.scene.snap && state.scene.snapMode !== "objects";
    const firstBars = sectionBars(state.scene.sections);
    for (const [i, [origin, beat, meter]] of state.scene.sections.entries()) {
      if (beat <= 0 || !meter) continue;
      const begin = Math.max(start, i ? origin : start);
      const until = Math.min(end, state.scene.sections[i + 1]?.[0] ?? end);
      if (begin >= until) continue;
      const widthOfBar = beat * meter * zoom;
      const stride = widthOfBar < 10 ? meter * Math.ceil(24 / widthOfBar) :
        beat * zoom < 8 ? meter : 1;

      if (useSnap && state.scene.beats === 1 && state.scene.divisor > 1) {
        const step = beat / state.scene.divisor;
        if (step * zoom >= 6) {
          for (let tick = Math.ceil((begin - origin) / step); origin + tick * step < until; tick++) {
            if (tick % state.scene.divisor === 0) continue;
            const x = (origin + tick * step - start) * zoom;
            const color = snapColour(tick, state.scene.divisor);
            const height = snapTickHeights[reducedDivisor(tick, state.scene.divisor)] ?? 4;
            line(context2d, ruler ? color : gridLines.sub, x, ruler ? bottom - height : top, x, bottom);
          }
        }
      }

      const step = beat * stride;
      for (let n = Math.ceil((begin - origin) / step); origin + n * step < until; n++) {
        const tick = n * stride;
        const x = (origin + n * step - start) * zoom;
        const bar = !state.scene.editorBars?.length && tick % meter === 0;
        const longSnap = useSnap && state.scene.beats > 1 && tick % state.scene.beats === 0;
        const color = ruler ? snapColours[1] : bar || longSnap ? gridLines.bar : gridLines.beat;
        // v3 §5: bar numbers on the ruler, every few bars when bars are under 60 px wide, counted from
        // the section's first bar. A label that would run into the next section's first one is left out.
        const barIndex = Math.floor(tick / meter);
        const barNumber = firstBars[i] + barIndex;
        const every = Math.max(1, Math.ceil(60 / widthOfBar));
        const nextSection = state.scene.sections[i + 1]?.[0];
        const room = nextSection === undefined ? Infinity : (nextSection - start) * zoom - x;
        if (ruler && bar && ((barIndex % every) + every) % every === 0 && room >= String(barNumber).length * 8 + 8) {
          context2d.fillStyle = slateTimeline.rulerBar;
          context2d.fillText(String(barNumber), x + 5, 10);
        }
        // v3 §5: bar lines run the ruler's full height; beat ticks (and long snaps) are 12 px.
        line(context2d, color, x, ruler && !bar ? bottom - 12 : top, x, bottom);
      }
    }
    for (const [i, section] of (state.scene.editorBars ?? []).entries()) {
      const begin = Math.max(start, section.start);
      const until = Math.min(end, state.scene.editorBars[i + 1]?.start ?? end);
      const length = section.beat * section.numerator;
      const stride = Math.max(1, Math.ceil((ruler ? 60 : 10) / (length * zoom)));
      const step = length * stride;
      if (begin >= until || !Number.isFinite(step) || step <= 0) continue;
      for (let n = Math.ceil((begin - section.start) / step); section.start + n * step < until; n++) {
        const x = (section.start + n * step - start) * zoom;
        line(context2d, ruler ? slateTimeline.rulerBar : gridLines.bar, x, top, x, bottom);
        if (ruler) {
          context2d.fillStyle = slateTimeline.rulerBar;
          context2d.fillText(String(section.bar + n * stride), x + 5, 10);
        }
      }
    }
  }

  // osu!lazer WaveformGraph.cs + Timeline.cs palette:
  // Base #66ccff @ 20% alpha -> Mid #44aadd -> High #2299bb -> Low (bass) #ddffff.
  function lazerWaveColour(low, mid, high) {
    let r = 102, g = 204, b = 255, a = 0.20;
    const blend = (tr, tg, tb, t) => {
      const k = limit(t, 0, 1);
      r += (tr - r) * k;
      g += (tg - g) * k;
      b += (tb - b) * k;
      a += (1 - a) * k;
    };
    blend(68, 170, 221, mid);
    blend(34, 153, 187, high);
    blend(221, 255, 255, low);
    return `rgba(${Math.round(r)},${Math.round(g)},${Math.round(b)},${a.toFixed(3)})`;
  }

  function waveform(context2d, width, center, height, start, span) {
    const bins = state.scene.waveform;
    if (!bins.length || !state.view.duration) return;
    // Playback reads the song at position - offset, so the drawn song moves by the same offset.
    const songStart = start - (state.view.offset ?? 0);
    const step = 2;
    let prevX = null;
    let prevL = 0;
    let prevR = 0;
    for (let x = 0; x <= width; x += step) {
      const t0 = songStart + x * span / width;
      const t1 = songStart + (x + step) * span / width;
      const a = Math.max(0, Math.floor(t0 / state.view.duration * bins.length));
      const b = Math.min(bins.length, Math.max(a + 1, Math.ceil(t1 / state.view.duration * bins.length)));
      if (a >= bins.length || t1 < 0) {
        prevX = null;
        continue;
      }
      let ampL = 0, ampR = 0, low = 0, mid = 0, high = 0, count = 0;
      for (let i = a; i < b; i++) {
        const bin = bins[i];
        ampL += bin[4] ?? bin[0];
        ampR += bin[5] ?? bin[0];
        low += bin[1];
        mid += bin[2];
        high += bin[3];
        count++;
      }
      if (!count) {
        prevX = null;
        continue;
      }
      ampL = limit(ampL / count, 0, 1) * height + 0.75;
      ampR = limit(ampR / count, 0, 1) * height + 0.75;
      if (prevX !== null) {
        context2d.fillStyle = lazerWaveColour(low / count, mid / count, high / count);
        context2d.beginPath();
        context2d.moveTo(prevX, center - prevL);
        context2d.lineTo(x, center - ampL);
        context2d.lineTo(x, center + ampR);
        context2d.lineTo(prevX, center + prevR);
        context2d.closePath();
        context2d.fill();
      }
      prevX = x;
      prevL = ampL;
      prevR = ampR;
    }
  }

  function paintSoon() {
    // While playing, the next playback frame paints anyway.
    if (!state.frame && !state.playbackFrame) state.frame = raf(() => { state.frame = 0; ctx.paint(); });
  }

  // Each view from the editor sets where the playhead is, and when. `drawn` is where the line was
  // just before this view.
  function anchorPlayhead(drawn) {
    if (!state.view.playing) return;
    const behind = drawn - state.view.position;
    // Held once at most: a line still behind on the next view goes to the report.
    const held = behind > 0 && behind <= playheadHold && !state.playheadAnchor.held;
    state.playheadAnchor = { position: held ? drawn : state.view.position, at: perf().now(), held };
  }

  // Where the playhead is drawn now: the last report moved on at the playback speed while playing.
  // While cycling it waits at the right locator for the editor to wrap.
  function playheadPosition() {
    let position = state.view.position;
    if (state.view.playing && Number.isFinite(position)) {
      const elapsed = Math.min(Math.max(0, perf().now() - state.playheadAnchor.at), playheadReach);
      position = state.playheadAnchor.position + elapsed * (state.view.speed || 1);
      if (state.view.cycle && state.view.left < state.view.right && state.playheadAnchor.position <= state.view.right) {
        position = Math.min(position, state.view.right);
      }
      if (Number.isFinite(state.view.duration)) position = Math.min(position, state.view.duration);
    }
    return position;
  }

  // Paints every frame while playing; stops, with one last paint, when playback stops.
  function animatePlayback() {
    if (state.playbackFrame) return;
    state.playbackFrame = raf(function step() {
      state.playbackFrame = 0;
      if (!state.scene || !state.view.playing || state.view.fault) {
        // Centre scrolled the view between the editor's views: tell it where the view ended up.
        if (state.scene && !state.view.fault && state.centreSentStart !== null) ctx.changeView(() => state.scene.viewStart);
        state.centreSentStart = null;
        paintSoon();
        return;
      }
      ctx.followPlayhead(true);
      ctx.paint();
      state.playbackFrame = raf(step);
    });
  }

  function fillHitDiamond(context2d, x, y) {
    context2d.beginPath();
    context2d.moveTo(x, y - 8);
    context2d.lineTo(x + 8, y);
    context2d.lineTo(x, y + 8);
    context2d.lineTo(x - 8, y);
    context2d.closePath();
    context2d.fill();
  }

  // The outline of a hit diamond, `size` px from its centre to each point (a filled one is 6).
  function strokeHitDiamond(context2d, x, y, size) {
    context2d.beginPath();
    context2d.moveTo(x, y - size);
    context2d.lineTo(x + size, y);
    context2d.lineTo(x, y + size);
    context2d.lineTo(x - size, y);
    context2d.closePath();
    context2d.lineWidth = 1.5;
    context2d.stroke();
  }

  // The combo colour of an object, or readout white without combo data.
  function comboOf(object) {
    const [number, colour] = state.scene.combos?.objects?.[object] ?? [];
    return { number, colour: state.scene.combos?.colours?.[colour] ?? objectWhite };
  }

  function roundedTrack(context2d, left, top, width, height, corner) {
    context2d.beginPath();
    if (context2d.roundRect) context2d.roundRect(left, top, width, height, corner);
    else {
      // Older macOS and Linux WebViews have no roundRect(); arcTo() draws the same shape.
      const right = left + width;
      const bottom = top + height;
      context2d.moveTo(left + corner, top);
      context2d.arcTo(right, top, right, bottom, corner);
      context2d.arcTo(right, bottom, left, bottom, corner);
      context2d.arcTo(left, bottom, left, top, corner);
      context2d.arcTo(left, top, right, top, corner);
      context2d.closePath();
    }
  }

  // A slider's first and last nodes are full circles; repeats between them sit on the track.
  // Only a circle and a slider's head carry the combo number.
  function nodeRadii(points, diameter) {
    // Each slider's first and last node time, built once per object list.
    if (state.sliderEndCache?.points !== points) {
      const ends = new Map();
      for (const [kind, time, object] of points) {
        if (kind !== "slider") continue;
        const known = ends.get(object);
        ends.set(object, known ? [Math.min(known[0], time), Math.max(known[1], time)] : [time, time]);
      }
      state.sliderEndCache = { points, ends };
    }
    const sliderEnds = state.sliderEndCache.ends;
    return (kind, time, object) => {
      const ends = kind === "slider" ? sliderEnds.get(object) : null;
      const repeat = Boolean(ends && time !== ends[0] && time !== ends[1]);
      const tail = Boolean(ends && time === ends[1] && ends[1] !== ends[0]);
      return { radius: repeat ? diameter * 0.66 / 2 : diameter / 2, head: !ends || time === ends[0], repeat, tail };
    };
  }

  function tintedHitCircle(skin, colour) {
    let cached = skin.tinted.get(colour);
    if (cached) return cached;
    const src = skin.hitcircle;
    const w = src.naturalWidth || src.width;
    const h = src.naturalHeight || src.height;
    const off = env.document.createElement("canvas");
    off.width = w;
    off.height = h;
    const octx = off.getContext("2d");
    // v3 timeline-spec §9: solid combo-colour disc clipped to hitcircle alpha,
    // then hitcircle drawn on top with overlay blend at 50% for shading.
    octx.drawImage(src, 0, 0);
    octx.globalCompositeOperation = "source-in";
    octx.fillStyle = colour;
    octx.fillRect(0, 0, w, h);
    octx.globalCompositeOperation = "overlay";
    octx.globalAlpha = 0.5;
    octx.drawImage(src, 0, 0);
    octx.globalAlpha = 1;
    octx.globalCompositeOperation = "destination-in";
    octx.drawImage(src, 0, 0);
    octx.globalCompositeOperation = "source-over";
    if (skin.tinted.size > 64) skin.tinted.clear();
    skin.tinted.set(colour, off);
    return off;
  }

  function drawSkinImage(context2d, img, cx, cy, scale) {
    const w = (img.naturalWidth || img.width) * scale;
    const h = (img.naturalHeight || img.height) * scale;
    context2d.drawImage(img, cx - w / 2, cy - h / 2, w, h);
  }

  function drawSkinNumber(context2d, skin, cx, cy, scale, number) {
    const overlap = 4; // @2x pixels (osu!'s default HitCircleOverlap = 2 px at 1x)
    const digits = String(number);
    let totalWidth = overlap;
    for (let i = 0; i < digits.length; i++) {
      const g = skin.digits?.[Number(digits[i])];
      if (!g) return;
      totalWidth += (g.naturalWidth || g.width) - overlap;
    }
    let x = cx - (totalWidth * scale) / 2;
    for (let i = 0; i < digits.length; i++) {
      const g = skin.digits[Number(digits[i])];
      const gw = g.naturalWidth || g.width;
      const gh = g.naturalHeight || g.height;
      context2d.drawImage(g, x, cy - (gh * scale) / 2, gw * scale, gh * scale);
      x += (gw - overlap) * scale;
    }
  }

  function drawSkinCircle(context2d, skin, cx, cy, radius, colour, combo, arrow = false) {
    const srcW = skin.hitcircle.naturalWidth || skin.hitcircle.width;
    if (!(srcW > 0) || !(radius > 0)) return;
    const side = (radius * 2) / (skin.circleFill || 0.92);
    const scale = side / srcW;
    drawSkinImage(context2d, tintedHitCircle(skin, colour), cx, cy, scale);
    drawSkinImage(context2d, skin.overlay, cx, cy, scale);
    if (arrow && skin.reverseArrow) drawSkinImage(context2d, skin.reverseArrow, cx, cy, scale);
    else if (combo > 0) drawSkinNumber(context2d, skin, cx, cy, scale * 0.8, combo);
  }

  function drawGameplay(context2d, gameplay, xOf, y, width, diameter) {
    if (!gameplay) return;
    const skin = state.skinTextures?.ready ? state.skinTextures : null;
    if (skin) {
      const radius = diameter / 2;
      context2d.lineWidth = 2;
      for (const [kind, start, end, object] of gameplay.spans) {
        const left = xOf(start);
        const right = xOf(end);
        if (right < -objectReach || left > width + objectReach) continue;
        if (kind === "slider") {
          const track = diameter * 0.66;
          const trackW = Math.max(2, right - left);
          const top = y - track / 2;
          context2d.fillStyle = trackShade(comboOf(object).colour);
          roundedTrack(context2d, left, top, trackW, track, Math.min(track, trackW) / 2);
          context2d.fill();
          const inner = Math.max(0, trackW - 2);
          context2d.strokeStyle = "#ffffffd9";
          roundedTrack(context2d, left + 1, top + 1, inner, Math.max(0, track - 2), Math.min(Math.max(0, track - 2), inner) / 2);
          context2d.stroke();
          continue;
        }
        context2d.fillStyle = objectWhite;
        const from = Math.max(0, left);
        const to = Math.min(width, right);
        context2d.fillRect(from, y - 2.5, Math.max(1, to - from), 5);
      }
      const radiusOf = nodeRadii(gameplay.points, diameter);
      const points = gameplay.points;
      // Draw slider tails and repeats first so slider heads with combo digits sit cleanly on top.
      for (let at = points.length - 1; at >= 0; at--) {
        const [kind, time, object] = points[at];
        const info = radiusOf(kind, time, object);
        if (info.head) continue;
        const x = xOf(time);
        if (x < -objectReach || x > width + objectReach) continue;
        const { colour } = comboOf(object);
        const r = info.tail ? radius * 0.8 : radius * 0.55;
        drawSkinCircle(context2d, skin, x, y, r, colour, 0, info.repeat);
      }
      for (const [at, [kind, time, object]] of points.entries()) {
        const info = radiusOf(kind, time, object);
        if (!info.head) continue;
        const x = xOf(time);
        if (x < -objectReach || x > width + objectReach) continue;
        const next = points[at + 1];
        const nextX = next ? xOf(next[1]) : Infinity;
        if (next && radiusOf(next[0], next[1], next[2]).head && nextX - x < 2) continue;
        const { number, colour } = comboOf(object);
        drawSkinCircle(context2d, skin, x, y, radius, colour, number || 0, false);
      }
      context2d.lineWidth = 1;
      return;
    }
    context2d.lineWidth = 2;
    for (const [kind, start, end, object] of gameplay.spans) {
      const left = xOf(start);
      const right = xOf(end);
      if (right < -objectReach || left > width + objectReach) continue;
      if (kind === "slider") {
        const track = diameter * 0.66;
        const top = y - track / 2;
        context2d.fillStyle = trackShade(comboOf(object).colour);
        roundedTrack(context2d, left, top, right - left, track, Math.min(track, right - left) / 2);
        context2d.fill();
        // A 2 px white edge at 85 % inside the track.
        const inner = Math.max(0, right - left - 2);
        context2d.strokeStyle = "#ffffffd9";
        roundedTrack(context2d, left + 1, top + 1, inner, track - 2, Math.min(track - 2, inner) / 2);
        context2d.stroke();
        continue;
      }
      context2d.fillStyle = objectWhite;
      const from = Math.max(0, left);
      const to = Math.min(width, right);
      context2d.fillRect(from, y - 2.5, Math.max(1, to - from), 5);
    }
    const radiusOf = nodeRadii(gameplay.points, diameter);
    // Digits 16 px on a 34 px circle, smaller with it in a short Song row.
    const digitSize = Math.max(8, Math.round(diameter * 0.47));
    context2d.font = "600 " + digitSize + "px Inter, system-ui, sans-serif";
    context2d.textAlign = "center";
    context2d.textBaseline = "middle";
    context2d.lineJoin = "round";
    const points = gameplay.points;
    for (const [at, [kind, time, object]] of points.entries()) {
      const x = xOf(time);
      if (x < -objectReach || x > width + objectReach) continue;
      // Later objects draw on top: one under 2 px from the next is hidden by it, so it is not drawn.
      const next = points[at + 1];
      const nextX = next ? xOf(next[1]) : Infinity;
      if (nextX - x < 2) continue;
      const { number, colour } = comboOf(object);
      const { radius, head } = radiusOf(kind, time, object);
      context2d.lineWidth = 2;
      context2d.fillStyle = colour;
      context2d.strokeStyle = "#ffffff";
      context2d.beginPath();
      context2d.arc(x, y, radius, 0, Math.PI * 2);
      context2d.fill();
      context2d.stroke();
      // The combo number on a circle and a slider head, where the next circle leaves it in view, with a
      // dark outline so it reads on yellow and green.
      const covered = next && nextX - radiusOf(next[0], next[1], next[2]).radius < x + digitSize / 3;
      if (head && number && !covered) {
        context2d.lineWidth = 3;
        context2d.strokeStyle = "rgba(0,0,0,.55)";
        context2d.strokeText(String(number), x, y);
        context2d.fillStyle = "#ffffff";
        context2d.fillText(String(number), x, y);
      }
    }
    context2d.textAlign = "start";
    context2d.textBaseline = "alphabetic";
    context2d.lineWidth = 1;
  }

  // timeline-spec "Overview strip": the song's objects in 300 columns, each drawn as a 5 px diamond
  // whose strength (30 to 100 %) shows how many objects fall in it. Built once per object list and width.
  function overviewBins(gameplay, width, duration) {
    if (!state.overviewBinCache || state.overviewBinCache.gameplay !== gameplay ||
        state.overviewBinCache.width !== width || state.overviewBinCache.duration !== duration) {
      const counts = new Array(overviewColumns).fill(0);
      // Each object once, at its start: a circle, a slider's head, a spinner's start.
      const starts = new Map();
      for (const [, time, object] of gameplay?.points ?? []) if (!starts.has(object)) starts.set(object, time);
      for (const [kind, time, , object] of gameplay?.spans ?? []) if (kind === "spinner" && !starts.has(object)) starts.set(object, time);
      for (const time of starts.values()) {
        if (time < 0 || time > duration) continue;
        counts[Math.min(overviewColumns - 1, Math.floor(time / duration * overviewColumns))]++;
      }
      const most = Math.max(1, ...counts);
      const bins = counts.flatMap((count, column) => count ? [[(column + 0.5) * width / overviewColumns, 0.3 + 0.7 * count / most]] : []);
      state.overviewBinCache = { gameplay, width, duration, bins };
    }
    return state.overviewBinCache.bins;
  }

  function diamond(context2d, x, y, half) {
    context2d.beginPath();
    context2d.moveTo(x, y - half);
    context2d.lineTo(x + half, y);
    context2d.lineTo(x, y + half);
    context2d.lineTo(x - half, y);
    context2d.closePath();
  }

  function drawOverview({ ctx: context2d, width, height }, start, visible) {
    context2d.fillStyle = slateTimeline.overview;
    context2d.fillRect(0, 0, width, height);
    const duration = state.view.duration;
    if (!(duration > 0) || width <= 0) return;
    const middle = height / 2;
    for (const [x, strength] of overviewBins(state.scene.gameplay, width, duration)) {
      context2d.fillStyle = "rgba(196,196,196," + strength.toFixed(2) + ")";
      diamond(context2d, x, middle, 3.5);
      context2d.fill();
    }
    // The viewport box: 1 px --editor-text, radius 2, at least 8 px wide.
    // The box stays inside the strip: its 1 px edge is drawn within its width.
    const boxWidth = Math.min(width, Math.max(8, visible * width / duration));
    const boxLeft = Math.max(0, Math.min(start * width / duration, width - boxWidth));
    context2d.fillStyle = "rgba(241,241,241,.06)";
    context2d.strokeStyle = "#f1f1f1";
    roundedTrack(context2d, boxLeft + 0.5, 3.5, boxWidth - 1, Math.max(0, height - 7), 2);
    context2d.fill();
    context2d.stroke();
    // Problems on this difficulty: 7 px --editor-warning diamonds with a 1 px --editor-panel edge.
    context2d.fillStyle = "#efc16a";
    context2d.strokeStyle = slateTimeline.overview;
    const drawnProblems = new Set();
    for (const problem of state.view.outputProblems ?? []) {
      const x = Math.round(problem.time * width / duration);
      if (problem.timeline !== state.scene.path || drawnProblems.has(x)) continue;
      drawnProblems.add(x);
      diamond(context2d, problem.time * width / duration, middle, 5);
      context2d.fill();
      context2d.stroke();
    }
    // Bookmarks on this difficulty: crisp cyan ticks along the top edge of the overview strip.
    context2d.fillStyle = "#79d7dc";
    for (const mark of state.scene.bookmarks ?? []) {
      const bx = mark.time * width / duration;
      if (bx < 0 || bx > width) continue;
      context2d.fillRect(Math.round(bx) - 1, 0, 2, 7);
    }
    const playX = playheadPosition() * width / duration;
    line(context2d, slateTimeline.playhead, playX, 0, playX, height);
  }

  function drawLocatorFlags(context2d, width, left, right, cycling) {
    const flags = ctx.locatorFlags(width, left, right);
    context2d.font = "600 10px Inter, system-ui, sans-serif";
    for (const [side, letter] of [["left", "L"], ["right", "R"]]) {
      const { x, from } = flags[side];
      // The flag's box runs from `from`; a flag pushed off its line keeps to its own box.
      const pointsRight = from >= x;
      const [base, tip] = pointsRight ? [from, from + locatorFlagWidth] : [from + locatorFlagWidth, from];
      context2d.fillStyle = cycling ? slateTimeline.cycle : slateTimeline.locator;
      context2d.beginPath();
      context2d.moveTo(base, 0);
      context2d.lineTo(tip, 0);
      context2d.lineTo(tip + (pointsRight ? -3 : 3), locatorFlagHeight / 2);
      context2d.lineTo(tip, locatorFlagHeight);
      context2d.lineTo(base, locatorFlagHeight);
      context2d.closePath();
      context2d.fill();
      context2d.fillStyle = slateTimeline.ruler;
      context2d.fillText(letter, from + (pointsRight ? 3 : 5), 10);
    }
  }

  function drawBookmarks(context2d, marks, start, zoom, width, height = rulerBandHeight + bookmarkFlagHeight) {
    const visible = marks.map(mark => [mark, (mark.time - start) * zoom])
      .filter(([, x]) => x >= 0 && x <= width);
    if (!visible.length) return;
    context2d.save?.();
    context2d.font = "600 11px Inter, system-ui, sans-serif";
    const flagHeight = 16;
    for (const [i, [mark, x]] of visible.entries()) {
      const sx = Math.round(x);
      if (height > rulerBandHeight) {
        context2d.fillStyle = "rgba(121,215,220,.30)";
        context2d.fillRect(sx, rulerBandHeight, 1, height - rulerBandHeight);
      }
      context2d.fillStyle = "#79d7dc";
      context2d.fillRect(sx, rulerBandHeight, 2, bookmarkFlagHeight);
      const nextX = visible[i + 1]?.[1] ?? width;
      if (mark.label) {
        const measured = context2d.measureText?.(mark.label)?.width ?? (mark.label.length * 6.5);
        const naturalWidth = Math.min(180, Math.ceil(measured) + 12);
        const spaceRight = nextX - sx - 6;
        const flipLeft = sx + naturalWidth > width - 4 && sx > naturalWidth + 4;
        const pillWidth = flipLeft
          ? Math.min(naturalWidth, sx - 4)
          : Math.min(naturalWidth, Math.max(24, spaceRight));
        const flagX = flipLeft ? sx - pillWidth : sx + 2;
        if (pillWidth > 24) {
          context2d.fillStyle = "rgba(15,36,40,.94)";
          context2d.fillRect(flagX, rulerBandHeight, pillWidth, flagHeight);
          context2d.fillStyle = "#79d7dc";
          context2d.fillRect(flagX, rulerBandHeight, pillWidth, 1);
          context2d.fillRect(flagX, rulerBandHeight + flagHeight - 1, pillWidth, 1);
          context2d.fillRect(flipLeft ? flagX : flagX + pillWidth - 1, rulerBandHeight, 1, flagHeight);
          context2d.fillStyle = "#e6fbff";
          context2d.fillText(mark.label, flagX + 5, rulerBandHeight + 12, pillWidth - 8);
        } else {
          context2d.fillStyle = "#79d7dc";
          context2d.fillRect(flipLeft ? sx - 10 : sx, rulerBandHeight, 10, 6);
        }
      } else {
        const flipLeft = sx + 12 > width;
        const dir = flipLeft ? -1 : 1;
        if (context2d.beginPath && context2d.moveTo && context2d.lineTo && context2d.closePath && context2d.fill) {
          context2d.fillStyle = "#79d7dc";
          context2d.beginPath();
          context2d.moveTo(sx + (flipLeft ? 0 : 2), rulerBandHeight);
          context2d.lineTo(sx + (flipLeft ? 0 : 2) + dir * 11, rulerBandHeight + 5);
          context2d.lineTo(sx + (flipLeft ? 0 : 2), rulerBandHeight + 11);
          context2d.closePath();
          context2d.fill();
        } else {
          context2d.fillStyle = "#79d7dc";
          context2d.fillRect(flipLeft ? sx - 10 : sx, rulerBandHeight, 10, 6);
        }
      }
    }
    context2d.restore?.();
  }

  function paint() {
    if (!state.scene || $("workspace").hidden || $("editor-arrange").hidden) return;
    const { grid, ruler, controller, overview } = elements;
    const rowHeight = state.scene.rowHeight;
    const contentHeight = Math.max($("scroll").clientHeight, state.laneRows.length * rowHeight);
    const scrollTop = $("scroll").scrollTop;
    const canvasHeight = Math.min(contentHeight, $("scroll").clientHeight);
    grid.style.height = canvasHeight + "px";
    const g = prepare(grid, canvasHeight);
    const r = prepare(ruler);
    const volumePanel = $("volume-controller-panel");
    const v = volumePanel.hidden || volumePanel.dataset?.layoutBarHidden !== undefined ||
      (volumePanel.dataset?.bottom ?? "volume") !== "volume" ? null : prepare(controller);
    const o = prepare(overview);
    const { start, zoom } = { start: state.scene.viewStart, zoom: state.scene.zoom };
    const xOf = (time) => (time - start) * zoom;
    const end = start + g.width / zoom;
    const range = state.scene.range;
    if (v) {
      v.ctx.fillStyle = slateTimeline.volume;
      v.ctx.fillRect(0, 0, v.width, v.height);
      v.ctx.font = '10px Inter, system-ui, sans-serif';
      for (const percent of [100, 75, 50, 25, 0]) {
        const y = v.height - 1 - percent * (v.height - 3) / 100;
        const major = percent === 100 || percent === 50 || percent === 0;
        line(v.ctx, major ? slateTimeline.beatLine : "rgba(255,255,255,.04)", 0, y, v.width, y);
        v.ctx.fillStyle = "#a3a3a3";
        const showLabel = major ? v.height >= 64 : v.height >= 96;
        if (showLabel) v.ctx.fillText(String(percent), 5, percent === 0 ? y - 3 : y + 11);
      }
      if (state.volumeHoverTime != null) {
        const hx = xOf(state.volumeHoverTime);
        if (hx >= -8 && hx <= v.width + 8) {
          v.ctx.fillStyle = "rgba(119,189,250,.14)";
          v.ctx.fillRect(hx - 4.5, 0, 9, v.height);
        }
      }
    }
    for (let row = Math.floor(scrollTop / rowHeight); row <= Math.ceil((scrollTop + g.height) / rowHeight); row++) {
      const y = row * rowHeight - scrollTop;
      const lane = state.laneRows[row];
      g.ctx.fillStyle = lane === state.view.activeLane ? slateTimeline.activeRow : row % 2 ? slateTimeline.alternateRow : slateTimeline.arrange;
      if (lane === undefined) g.ctx.fillStyle = slateTimeline.arrange;
      else if (typeof lane === "string") g.ctx.fillStyle = slateTimeline.group;
      g.ctx.fillRect(0, y, g.width, rowHeight);
      const tint = typeof lane === "number" ? ctx.laneTint(state.scene.lanes?.[lane], lane === state.view.activeLane) : null;
      if (tint) {
        g.ctx.fillStyle = tint;
        g.ctx.fillRect(0, y, g.width, rowHeight);
      }
      if (lane !== undefined) line(g.ctx, slateTimeline.divider, 0, y + rowHeight - 0.5, g.width, y + rowHeight - 0.5);
    }
    if (range) {
      g.ctx.fillStyle = "#77bdfa35";
      const width = xOf(range[1]) - xOf(range[0]);
      if (state.markedRange?.path === state.scene.path) {
        for (const row of state.markedRange.rows) {
          for (const position of state.laneVisualPositions.get(row) ?? []) {
            g.ctx.fillRect(xOf(range[0]), position * rowHeight - scrollTop, width, rowHeight);
          }
        }
      } else {
        g.ctx.fillRect(xOf(range[0]), 0, width, g.height);
      }
    }
    drawTimingGrid(g.ctx, start, end, zoom, 0, g.height);
    if (state.volumeHoverTime != null) {
      const hx = xOf(state.volumeHoverTime);
      if (hx >= -8 && hx <= g.width + 8) {
        g.ctx.fillStyle = "rgba(119,189,250,.14)";
        g.ctx.fillRect(hx - 4.5, 0, 9, g.height);
      }
    }
    const problemTimes = new Set();
    for (const problem of state.view.outputProblems ?? []) {
      if (problem.timeline === state.scene.path) problemTimes.add(Math.round(problem.time));
    }
    if (problemTimes.size) {
      g.ctx.fillStyle = "rgba(232,107,114,.12)";
      for (const t of problemTimes) {
        const px = xOf(t);
        if (px >= -8 && px <= g.width + 8) g.ctx.fillRect(px - 4.5, 0, 9, g.height);
      }
    }
    let lastObjectPixel = null;
    for (const [time, volume, object, hits] of state.scene.moments) {
      const x = xOf(time);
      if (x < -8 || x > g.width + 8) continue;
      const objectPixel = Math.round(x);
      if (object && state.view.objectLines && objectPixel !== lastObjectPixel) {
        line(g.ctx, "#62d6cf65", objectPixel, 0, objectPixel, g.height);
        lastObjectPixel = objectPixel;
      }
      const colour = volumeColour(volume);
      if (v) {
        const volumeHeight = limit(volume, 0, 100) / 100 * (v.height - 3);
        const anySelected = hits.some(([, selected]) => selected);
        const topY = v.height - volumeHeight;
        v.ctx.fillStyle = colour;
        v.ctx.fillRect(x - 2.5, topY, 5, volumeHeight);
        // Bright 5x5 top cap (white when selected), matching 3.1.4 VolumeLane.
        v.ctx.fillStyle = anySelected ? "#ffffff" : colour;
        v.ctx.fillRect(x - 2.5, Math.max(0, topY - 2), 5, 5);
        // White horizontal ticks for any per-hit volumes that differ from the moment's shared volume.
        for (const [, , , hitVolume] of hits) {
          if (hitVolume != null && hitVolume !== volume) {
            const tickY = v.height - limit(hitVolume, 0, 100) / 100 * (v.height - 3);
            v.ctx.fillStyle = "#ffffff";
            v.ctx.fillRect(x - 3, tickY - 0.5, 6, 1.5);
          }
        }
      }
      const hasProblem = problemTimes.has(Math.round(time));
      for (const [row, selected, listenMuted, hitVolume] of hits) {
        for (const visibleRow of state.laneVisualPositions.get(row) ?? []) {
          const y = (visibleRow + 0.5) * rowHeight - scrollTop;
          if (y < -8 || y > g.height + 8) continue;
          g.ctx.fillStyle = listenMuted ? "#858585" : volumeColour(hitVolume ?? volume);
          fillHitDiamond(g.ctx, x, y);
          if (selected || hasProblem) {
            g.ctx.lineWidth = 2;
            g.ctx.strokeStyle = selected ? "#ffffff" : "#e86b72";
            g.ctx.stroke();
            g.ctx.lineWidth = 1;
          }
        }
      }
      if (state.previewParents.size) {
        // Over the main lane's own hits: a folded main lane outlines its layers' hits in their volume
        // colours, so they show volume and can't pass for muted hits; several at one moment nest, each
        // a little larger, so every colour shows. A folded folder shows its lanes' hits faint.
        const nested = new Map();
        for (const [row, , listenMuted, hitVolume] of hits) {
          const parent = state.previewParents.get(row);
          if (parent === undefined) continue;
          const y = (state.lanePositions.get(parent) + 0.5) * rowHeight - scrollTop;
          if (y < -12 || y > g.height + 12) continue;
          g.ctx.save();
          const colour = listenMuted ? "#858585" : volumeColour(hitVolume ?? volume);
          if (typeof parent === "number") {
            const inside = nested.get(parent) ?? 0;
            nested.set(parent, inside + 1);
            g.ctx.strokeStyle = colour;
            strokeHitDiamond(g.ctx, x, y, 9.5 + 2.5 * inside);
          } else {
            g.ctx.globalAlpha *= 0.3;
            g.ctx.fillStyle = colour;
            fillHitDiamond(g.ctx, x, y);
          }
          g.ctx.restore();
        }
      }
    }
    if (state.drag?.surface === grid && state.drag.lastPoint && ["Select", "Range"].includes(state.view.tool)) {
      const x = state.drag.firstPoint.x;
      const y = state.drag.firstPoint.y - scrollTop;
      const width = state.drag.lastPoint.x - x;
      const height = state.drag.lastPoint.y - state.drag.firstPoint.y;
      if ((state.view.tool === "Range" || !state.drag.hit) && Math.hypot(width, height) > 3) {
        g.ctx.fillStyle = "#77bdfa14";
        g.ctx.fillRect(x, y, width, height);
        g.ctx.strokeStyle = "#77bdfa";
        g.ctx.strokeRect(x, y, width, height);
      }
    }
    if (v && state.scene.rampPreview?.length) {
      v.ctx.strokeStyle = "#62d6cf";
      v.ctx.beginPath();
      for (const [i, [time, volume]] of state.scene.rampPreview.entries()) {
        const x = xOf(time);
        const y = v.height - volume / 100 * (v.height - 3);
        if (i === 0) v.ctx.moveTo(x, y);
        else v.ctx.lineTo(x, y);
      }
      v.ctx.stroke();
    }
    r.ctx.fillStyle = slateTimeline.waveform;
    r.ctx.fillRect(0, 0, r.width, r.height);
    const waveformMiddle = (r.height + rulerBandHeight) / 2;
    const waveAmplitude = Math.min(state.waveformHeight, Math.max(0, r.height - rulerBandHeight)) / 2;
    if (state.waveformShown && waveAmplitude > 0) waveform(r.ctx, r.width, waveformMiddle, waveAmplitude, start, r.width / zoom);
    if (r.height > rulerBandHeight) drawTimingGrid(r.ctx, start, end, zoom, rulerBandHeight, r.height, false);
    const ngOverlay = visibleViewerOverlay(
      ctx.ui()?.nodeGraphViewer?.(), start, end, xOf, rulerBandHeight + 2, Math.max(0, r.height - rulerBandHeight - 4),
    );
    if (ngOverlay?.kind === "curve" && ngOverlay.points.length) {
      r.ctx.save();
      r.ctx.strokeStyle = "#f0aa70";
      r.ctx.lineWidth = 1.5;
      r.ctx.beginPath();
      for (const [i, [px, py]] of ngOverlay.points.entries()) {
        if (i === 0) r.ctx.moveTo(px, py);
        else r.ctx.lineTo(px, py);
      }
      r.ctx.stroke();
      r.ctx.restore();
    } else if (ngOverlay?.kind === "events" && ngOverlay.markers.length) {
      r.ctx.save();
      r.ctx.fillStyle = "#62d6cf";
      r.ctx.strokeStyle = "#62d6cf";
      for (const m of ngOverlay.markers) {
        line(r.ctx, "#62d6cf", m.x, rulerBandHeight + 2, m.x, r.height - 2);
        r.ctx.fillRect(m.x - 2, rulerBandHeight + 2, 4, 4);
      }
      r.ctx.restore();
    }
    drawGameplay(r.ctx, state.scene.gameplay, xOf, waveformMiddle, r.width, Math.max(0, Math.min(objectSize, r.height - rulerBandHeight)));
    // v3 §5: the 40 px ruler, #0c0c0c with a 1 px --editor-selected bottom edge, bar numbers 13 px/600.
    r.ctx.fillStyle = slateTimeline.ruler;
    r.ctx.fillRect(0, 0, r.width, rulerBandHeight);
    line(r.ctx, slateTimeline.rulerEdge, 0, rulerBandHeight - 0.5, r.width, rulerBandHeight - 0.5);
    r.ctx.font = "600 13px Inter, system-ui, sans-serif";
    r.ctx.textBaseline = "top";
    drawTimingGrid(r.ctx, start, end, zoom, 0, rulerBandHeight, true);
    r.ctx.textBaseline = "alphabetic";
    const preview = state.rulerDrag?.path === state.scene.path ? state.rulerDrag.preview : null;
    const [left, right, cycling] = preview ?? [state.view.left, state.view.right, state.view.cycle];
    const spanStart = xOf(Math.min(left, right));
    const spanWidth = xOf(Math.max(left, right)) - spanStart;
    if (cycling && left <= right) {
      // The design's loop region: a faint accent band under a warning-yellow bar.
      r.ctx.fillStyle = slateTimeline.loop;
      r.ctx.fillRect(spanStart, 0, spanWidth, rulerBandHeight - 1);
      r.ctx.fillStyle = slateTimeline.cycle;
      r.ctx.fillRect(spanStart, 1, spanWidth, 4);
    } else {
      const colour = left > right ? "#f0aa70" : slateTimeline.locator;
      r.ctx.fillStyle = colour + (cycling ? "46" : "22");
      r.ctx.fillRect(spanStart, 0, spanWidth, rulerBandHeight - 1);
      r.ctx.fillStyle = colour;
      r.ctx.fillRect(spanStart, 0, spanWidth, 2);
    }
    for (const time of [left, right]) {
      line(r.ctx, slateTimeline.locator, xOf(time), 0, xOf(time), r.height);
    }
    drawLocatorFlags(r.ctx, r.width, left, right, cycling && left <= right);
    drawBookmarks(r.ctx, state.scene.bookmarks ?? [], start, zoom, r.width, r.height);
    for (const mark of state.scene.bookmarks ?? []) {
      const bx = xOf(mark.time);
      if (bx < 0 || bx > g.width) continue;
      line(g.ctx, "rgba(121,215,220,.16)", bx, 0, bx, g.height);
    }
    const playhead = xOf(playheadPosition());
    line(g.ctx, slateTimeline.playhead, playhead, 0, playhead, g.height);
    line(r.ctx, slateTimeline.playhead, playhead, 0, playhead, r.height);
    state.rulerWarningMarkers = visibleWarningMarkers(state.view.outputProblems ?? [], state.scene.path, start, zoom, r.width);
    ctx.refreshRulerWarningTooltip();
    r.ctx.fillStyle = "#f0aa70";
    for (const marker of state.rulerWarningMarkers) {
      r.ctx.fillRect(marker.x - 3, r.height - 9, 6, 6);
    }
    if (v) line(v.ctx, slateTimeline.playhead, playhead, 0, playhead, v.height);
    drawOverview(o, start, g.width / zoom);
  }

  Object.assign(ctx, {
    prepare, line, sectionBars, drawTimingGrid, waveform, paintSoon,
    anchorPlayhead, playheadPosition, animatePlayback,
    fillHitDiamond, strokeHitDiamond, comboOf, roundedTrack, nodeRadii,
    drawGameplay, overviewBins, diamond, drawOverview, drawLocatorFlags,
    drawBookmarks, paint,
  });
}

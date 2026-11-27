// Dense Canvas 2D renderer for the timeline grid, waveform, gameplay objects, hit diamonds,
// volume lane, and song overview strip. Kept on Canvas for Chromium + WebKit performance.
import {
  bookmarkFlagHeight, gridLines, limit, locatorFlagHeight, locatorFlagWidth,
  objectReach, objectSize, objectWhite, overviewColumns, reducedDivisor,
  rulerBandHeight, slateTimeline, snapColour, snapColours, snapTickHeights,
  trackShade, visibleVolumeLanes, visibleWarningMarkers, volumeColour,
} from "./helpers.js";
import { visibleViewerOverlay } from "../lib/node-graph.js";
import { chordMarks, drawChordMarks, paintRoll, paintVelocity } from "../lib/rollPaint.js";

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
    return { ctx: context2d, width, height, scale };
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

  // Cubase-style musical power-of-two bar thinning (1, 2, 4, 8, 16, 32, 64...) when zoomed far out.
  function musicalBarStride(widthOfBar, minPx) {
    if (!(widthOfBar > 0)) return 1;
    let bars = 1;
    while (bars * widthOfBar < minPx && bars < 1024) bars *= 2;
    return bars;
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
      const stride = widthOfBar < 24 ? meter * musicalBarStride(widthOfBar, 28) :
        beat * zoom < 12 ? meter : 1;

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
      const every = musicalBarStride(widthOfBar, 60);
      for (let n = Math.ceil((begin - origin) / step); origin + n * step < until; n++) {
        const tick = n * stride;
        const x = (origin + n * step - start) * zoom;
        const bar = !state.scene.editorBars?.length && tick % meter === 0;
        const longSnap = useSnap && state.scene.beats > 1 && tick % state.scene.beats === 0;
        const color = ruler ? snapColours[1] : bar || longSnap ? gridLines.bar : gridLines.beat;
        // v3 §5 + Cubase musical thinning: bar numbers on power-of-two bar intervals (1, 2, 4, 8, 16...).
        const barIndex = Math.floor(tick / meter);
        const barNumber = firstBars[i] + barIndex;
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
      const stride = musicalBarStride(length * zoom, ruler ? 60 : 28);
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

  // High-detail un!stable-style 3-band frequency palette:
  // Low (bass/kicks) = vivid crimson/orange-red, Mid (snares/vocals) = crisp emerald/cyan-blue,
  // High (hi-hats/cymbals) = bright gold/ice-white, blended smoothly by band energy.
  // The three bands are quantized to 1/16 so each colour string is built once and reused.
  const waveLevels = 16;
  const waveColours = new Array((waveLevels + 1) ** 3);
  function lazerWaveColourIndex(low, mid, high) {
    const ql = Math.round(limit(low, 0, 1) * waveLevels);
    const qm = Math.round(limit(mid, 0, 1) * waveLevels);
    const qh = Math.round(limit(high, 0, 1) * waveLevels);
    const index = (ql * (waveLevels + 1) + qm) * (waveLevels + 1) + qh;
    waveColours[index] ??= blendWaveColour(ql / waveLevels, qm / waveLevels, qh / waveLevels);
    return index;
  }

  function lazerWaveColour(low, mid, high) {
    return waveColours[lazerWaveColourIndex(low, mid, high)];
  }

  function blendWaveColour(low, mid, high) {
    const total = low + mid + high;
    if (total <= 1e-4) return "rgba(110,160,195,0.280)";
    const wL = low ** 1.35;
    const wM = mid ** 1.35;
    const wH = high ** 1.35;
    const wSum = wL + wM + wH || 1;
    const r = (255 * wL + 32 * wM + 255 * wH) / wSum;
    const g = (62 * wL + 216 * wM + 244 * wH) / wSum;
    const b = (48 * wL + 206 * wM + 196 * wH) / wSum;
    const intensity = Math.max(low, mid, high);
    const a = 0.55 + 0.40 * limit(intensity, 0, 1);
    return `rgba(${Math.round(r)},${Math.round(g)},${Math.round(b)},${a.toFixed(3)})`;
  }

  const WAVE_POOL_SIZE = 32768;
  const WAVE_POOL_MASK = WAVE_POOL_SIZE - 1;

  function waveform(context2d, width, center, height, start, span) {
    const pyramid = state.scene.waveformPyramid;
    const legacyBins = state.scene.waveform;
    const duration = state.view.duration;
    if (!duration || width <= 0) return;

    const msPerPx = span / width;
    let bins = legacyBins;
    let isTyped = !Array.isArray(bins) && (bins instanceof Float32Array || Object.prototype.toString.call(bins) === "[object Float32Array]");
    let binCount = isTyped ? Math.floor(bins.length / 6) : (bins?.length ?? 0);
    let clipMask = null;
    let strideBins = 1;

    if (pyramid?.levels?.length && pyramid.baseCount > 0) {
      const binsPerPx = msPerPx * (pyramid.baseCount / duration);
      let chosen = pyramid.levels[0];
      for (let li = 1; li < pyramid.levels.length; li++) {
        const cand = pyramid.levels[li];
        if (binsPerPx / cand.strideBins >= 1) chosen = cand;
        else break;
      }
      bins = chosen.bins;
      binCount = chosen.count;
      clipMask = chosen.clipMask ?? null;
      strideBins = chosen.strideBins;
      isTyped = true;
    }
    if (!binCount) return;

    // Playback reads the song at position - offset, so the drawn song moves by the same offset.
    const songStart = start - (state.view.offset ?? 0);
    const cols = Math.ceil(width);
    const binScale = binCount / duration;
    if (!state.waveCoreTops || state.waveCoreTops.length < cols) {
      state.waveCoreTops = new Float32Array(cols);
      state.waveCoreHeights = new Float32Array(cols);
      state.wavePeakL = new Float32Array(cols);
      state.wavePeakR = new Float32Array(cols);
      state.waveClip = new Uint8Array(cols);
    }
    const coreTops = state.waveCoreTops;
    const coreHeights = state.waveCoreHeights;
    const colPeakL = state.wavePeakL;
    const colPeakR = state.wavePeakR;
    const colClip = state.waveClip;
    coreHeights.fill(0, 0, cols);
    colClip.fill(0, 0, cols);

    // Zero-GC typed-array column cache keyed by absolute song-time pixel column.
    let pool = state.wavePool;
    if (!pool) {
      pool = {
        bins: null,
        duration: 0,
        msPerPx: 0,
        height: 0,
        strideBins: 0,
        gen: 1,
        count: 0,
        keys: new Int32Array(WAVE_POOL_SIZE),
        slotGen: new Uint32Array(WAVE_POOL_SIZE),
        valid: new Uint8Array(WAVE_POOL_SIZE),
        peakL: new Float32Array(WAVE_POOL_SIZE),
        peakR: new Float32Array(WAVE_POOL_SIZE),
        rmsL: new Float32Array(WAVE_POOL_SIZE),
        rmsR: new Float32Array(WAVE_POOL_SIZE),
        colourIdx: new Int32Array(WAVE_POOL_SIZE),
        clipped: new Uint8Array(WAVE_POOL_SIZE),
      };
      state.wavePool = pool;
    }
    if (pool.bins !== bins || pool.duration !== duration || pool.msPerPx !== msPerPx ||
        pool.height !== height || pool.strideBins !== strideBins || pool.count > 20000) {
      pool.bins = bins;
      pool.duration = duration;
      pool.msPerPx = msPerPx;
      pool.height = height;
      pool.strideBins = strideBins;
      pool.gen = (pool.gen + 1) >>> 0;
      if (pool.gen === 0) {
        pool.slotGen.fill(0);
        pool.gen = 1;
      }
      pool.count = 0;
    }

    const absCol0 = Math.floor(songStart / msPerPx);
    const subPx = songStart - absCol0 * msPerPx;
    const useCache = Math.abs(subPx) < msPerPx * 0.5 || state.view.playing;
    let lastColour = "";
    let hasClip = false;

    for (let x = 0; x < cols; x++) {
      const absCol = absCol0 + x;
      let hit = false;
      let empty = false;
      let peakL = 0, peakR = 0, rmsL = 0, rmsR = 0, colourIdx = 0, clipped = 0;
      let emptySlot = -1;

      if (useCache) {
        let idx = (Math.imul(absCol, 2654435761) >>> 0) & WAVE_POOL_MASK;
        for (let probe = 0; probe < 64; probe++) {
          if (pool.slotGen[idx] !== pool.gen) {
            emptySlot = idx;
            break;
          }
          if (pool.keys[idx] === absCol) {
            hit = true;
            if (!pool.valid[idx]) {
              empty = true;
            } else {
              peakL = pool.peakL[idx];
              peakR = pool.peakR[idx];
              rmsL = pool.rmsL[idx];
              rmsR = pool.rmsR[idx];
              colourIdx = pool.colourIdx[idx];
              clipped = pool.clipped[idx];
            }
            break;
          }
          idx = (idx + 1) & WAVE_POOL_MASK;
        }
      }

      if (!hit) {
        const t0 = useCache ? absCol * msPerPx : songStart + x * msPerPx;
        const t1 = t0 + msPerPx;
        if (t1 < 0) {
          if (useCache && emptySlot >= 0) {
            pool.keys[emptySlot] = absCol;
            pool.slotGen[emptySlot] = pool.gen;
            pool.valid[emptySlot] = 0;
            pool.count++;
          }
          continue;
        }
        const a = Math.max(0, Math.floor(t0 * binScale));
        if (a >= binCount) break;
        const b = Math.min(binCount, Math.max(a + 1, Math.ceil(t1 * binScale)));

        let maxL = 0, maxR = 0, sumSqL = 0, sumSqR = 0;
        let wLow = 0, wMid = 0, wHigh = 0, wAmp = 0;
        let sumLow = 0, sumMid = 0, sumHigh = 0, count = 0;
        if (isTyped) {
          for (let i = a; i < b; i++) {
            const base = i * 6;
            const l = bins[base + 4];
            const r = bins[base + 5];
            const low = bins[base + 1];
            const mid = bins[base + 2];
            const high = bins[base + 3];
            if (l > maxL) maxL = l;
            if (r > maxR) maxR = r;
            sumSqL += l * l;
            sumSqR += r * r;
            const amp = l > r ? l : r;
            wLow += low * amp;
            wMid += mid * amp;
            wHigh += high * amp;
            wAmp += amp;
            sumLow += low;
            sumMid += mid;
            sumHigh += high;
            if (clipMask && (clipMask[i >> 3] & (1 << (i & 7))) !== 0) clipped = 1;
            count++;
          }
        } else {
          for (let i = a; i < b; i++) {
            const bin = bins[i];
            const l = bin[4] ?? bin[0];
            const r = bin[5] ?? bin[0];
            if (l > maxL) maxL = l;
            if (r > maxR) maxR = r;
            sumSqL += l * l;
            sumSqR += r * r;
            const amp = Math.max(l, r);
            wLow += bin[1] * amp;
            wMid += bin[2] * amp;
            wHigh += bin[3] * amp;
            wAmp += amp;
            sumLow += bin[1];
            sumMid += bin[2];
            sumHigh += bin[3];
            count++;
          }
        }
        if (!count) {
          if (useCache && emptySlot >= 0) {
            pool.keys[emptySlot] = absCol;
            pool.slotGen[emptySlot] = pool.gen;
            pool.valid[emptySlot] = 0;
            pool.count++;
          }
          continue;
        }

        peakL = limit(maxL, 0, 1) * height + 0.5;
        peakR = limit(maxR, 0, 1) * height + 0.5;
        const colLow = wAmp > 1e-4 ? wLow / wAmp : sumLow / count;
        const colMid = wAmp > 1e-4 ? wMid / wAmp : sumMid / count;
        const colHigh = wAmp > 1e-4 ? wHigh / wAmp : sumHigh / count;
        colourIdx = lazerWaveColourIndex(colLow, colMid, colHigh);
        rmsL = limit(Math.sqrt(sumSqL / count), 0, 1) * height * 0.72;
        rmsR = limit(Math.sqrt(sumSqR / count), 0, 1) * height * 0.72;
        if (useCache && emptySlot >= 0) {
          pool.keys[emptySlot] = absCol;
          pool.slotGen[emptySlot] = pool.gen;
          pool.valid[emptySlot] = 1;
          pool.peakL[emptySlot] = peakL;
          pool.peakR[emptySlot] = peakR;
          pool.rmsL[emptySlot] = rmsL;
          pool.rmsR[emptySlot] = rmsR;
          pool.colourIdx[emptySlot] = colourIdx;
          pool.clipped[emptySlot] = clipped;
          pool.count++;
        }
      }
      if (empty) continue;

      const colour = waveColours[colourIdx];
      if (colour !== lastColour) {
        context2d.fillStyle = colour;
        lastColour = colour;
      }
      context2d.fillRect(x, center - peakL, 1, Math.max(1, peakL + peakR));

      colPeakL[x] = peakL;
      colPeakR[x] = peakR;
      if (clipped) {
        colClip[x] = 1;
        hasClip = true;
      }

      const coreH = rmsL + rmsR;
      if (coreH >= 1.5) {
        coreTops[x] = center - rmsL;
        coreHeights[x] = coreH;
      }
    }

    // Subtle inner RMS/core highlight pass for depth without obscuring transients.
    context2d.fillStyle = "rgba(255,255,255,0.20)";
    for (let x = 0; x < cols; x++) {
      const h = coreHeights[x];
      if (h > 0) context2d.fillRect(x, coreTops[x], 1, h);
    }

    if (hasClip) {
      context2d.fillStyle = "#ff453a";
      for (let x = 0; x < cols; x++) {
        if (!colClip[x]) continue;
        context2d.fillRect(x, center - colPeakL[x], 1, 2);
        context2d.fillRect(x, Math.max(center, center + colPeakR[x] - 2), 1, 2);
      }
    }
  }

  // keepLayers: show() during playback with no new scene; layerKey() already holds every view field
  // paint() draws from, so a poll that only moved the position keeps the cached layers.
  function paintSoon(keepLayers = false) {
    // Something changed: the next frame paints every layer again instead of reusing the cached ones.
    if (!keepLayers) state.staticLayers = null;
    // While playing, the next playback frame paints anyway.
    if (!state.frame && !state.playbackFrame) state.frame = raf(() => { state.frame = 0; paintFrame(); });
  }

  // timeline-spec: while playing, only the playhead moves. A full paint keeps a copy of each canvas
  // without the playhead; the frames after it copy that back and draw the playhead alone, until the
  // view, scroll, zoom, size or anything paintSoon() hears about changes.
  function layerKey() {
    const scroll = $("scroll");
    const view = state.view;
    return [state.scene, state.scene.waveform, view.activeLane, view.objectLines, view.left, view.right, view.cycle, view.duration,
      view.offset, view.tool, !!view.songLoading, state.outputProblemList, state.scene.viewStart, state.scene.zoom, state.scene.rowHeight,
      scroll.scrollTop, scroll.clientHeight, elements.grid.clientWidth, elements.ruler.clientWidth,
      elements.ruler.clientHeight, elements.overview.clientWidth, elements.controller.clientHeight,
      env.window.devicePixelRatio || 1, env.document?.documentElement?.dataset?.uiScale ?? "100",
      state.laneRows, state.waveformShown, state.waveformHeight, state.soundFirst, ctx.ui()?.keysoundRoll?.() ?? null];
  }

  function sameKey(a, b) {
    if (!a || a.length !== b.length) return false;
    for (let i = 0; i < a.length; i++) if (!Object.is(a[i], b[i])) return false;
    return true;
  }

  function keepLayers(layers) {
    try {
      const kept = [];
      for (const [canvas, drawn] of layers) {
        if (!drawn) continue;
        const copy = state.layerCopies?.get(canvas) ?? env.document.createElement("canvas");
        if (copy.width !== canvas.width) copy.width = canvas.width;
        if (copy.height !== canvas.height) copy.height = canvas.height;
        const copyContext = copy.getContext?.("2d");
        if (typeof copyContext?.drawImage !== "function") return null;
        copyContext.globalCompositeOperation = "copy";
        copyContext.drawImage(canvas, 0, 0);
        (state.layerCopies ??= new Map()).set(canvas, copy);
        kept.push([canvas, copy, drawn]);
      }
      return kept;
    } catch {
      return null;
    }
  }

  // One playback frame: the cached layers and the playhead when nothing else changed, else a full paint.
  function paintFrame() {
    if (!state.scene || $("workspace").hidden || $("editor-arrange").hidden) return;
    const cached = state.staticLayers;
    if (!cached || !sameKey(cached.key, layerKey())) {
      ctx.paint();
      return;
    }
    const surfaces = {};
    for (const [canvas, copy, drawn] of cached.layers) {
      const context2d = canvas.getContext("2d");
      context2d.setTransform(1, 0, 0, 1, 0, 0);
      context2d.globalCompositeOperation = "copy";
      context2d.drawImage(copy, 0, 0);
      context2d.globalCompositeOperation = "source-over";
      context2d.setTransform(cached.scale, 0, 0, cached.scale, 0, 0);
      surfaces[drawn.name] = { ctx: context2d, width: drawn.width, height: drawn.height };
    }
    drawPlayheads(surfaces.g, surfaces.r, surfaces.v ?? null, surfaces.o);
  }

  // Subtle playback hitlighting on diamonds as the playhead crosses them (140 ms ease-out glow).
  // Drawn on top of the cached static layers in drawPlayheads() so it adds zero static-layer invalidation.
  function drawPlaybackHitLighting(g, position) {
    if (!state.view.playing || !g || !state.scene?.moments?.length || state.view.hitsoundsMuted) return;
    const moments = state.scene.moments;
    const windowMs = 140;
    const minTime = position - windowMs;
    // Binary search for the first moment >= minTime.
    let lo = 0, hi = moments.length;
    while (lo < hi) {
      const mid = (lo + hi) >>> 1;
      if (moments[mid][0] < minTime) lo = mid + 1;
      else hi = mid;
    }
    const start = state.scene.viewStart;
    const zoom = state.scene.zoom;
    const rowHeight = state.scene.rowHeight;
    const scrollTop = $("scroll").scrollTop;
    const lanes = state.scene.lanes ?? [];
    const anySolo = lanes.some(l => l?.solo);
    const ctx2d = g.ctx;
    ctx2d.save();
    for (let i = lo; i < moments.length; i++) {
      const [time, , , hits] = moments[i];
      if (time > position) break;
      const x = (time - start) * zoom;
      if (x < -16 || x > g.width + 16) continue;
      const t = 1 - (position - time) / windowMs;
      const ease = t * t;
      const haloSize = 8 + 4 * (1 - t);
      for (const [row, , listenMuted] of hits) {
        if (listenMuted) continue;
        const laneInfo = lanes[row];
        if (laneInfo?.muted || (anySolo && !laneInfo?.solo)) continue;
        const visibleRow = state.lanePositions.get(row);
        if (visibleRow === undefined) continue;
        const y = (visibleRow + 0.5) * rowHeight - scrollTop;
        if (y < -16 || y > g.height + 16) continue;
        // Soft expanding outer glow diamond
        ctx2d.fillStyle = `rgba(255, 255, 255, ${(0.28 * ease).toFixed(3)})`;
        diamond(ctx2d, x, y, haloSize);
        ctx2d.fill();
        // Crisp bright inner highlight on the diamond itself
        ctx2d.fillStyle = `rgba(255, 255, 255, ${(0.55 * ease).toFixed(3)})`;
        diamond(ctx2d, x, y, 7.5);
        ctx2d.fill();
      }
    }
    ctx2d.restore();
  }

  // The playhead on the grid, the ruler, the volume lane and the overview strip.
  function drawPlayheads(g, r, v, o) {
    const position = playheadPosition();
    // The glow belongs to hit diamonds; in Keysounds mode the piano roll has that canvas.
    if (g && !ctx.ui()?.keysoundRoll?.()) drawPlaybackHitLighting(g, position);
    const playhead = (position - state.scene.viewStart) * state.scene.zoom;
    if (g) line(g.ctx, slateTimeline.playhead, playhead, 0, playhead, g.height);
    if (r) line(r.ctx, slateTimeline.playhead, playhead, 0, playhead, r.height);
    if (v) line(v.ctx, slateTimeline.playhead, playhead, 0, playhead, v.height);
    const duration = state.view.duration;
    if (o && duration > 0 && o.width > 0) {
      const playX = position * o.width / duration;
      line(o.ctx, slateTimeline.playhead, playX, 0, playX, o.height);
    }
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
      paintFrame();
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

  function overviewVolumeHex(volume) {
    if (volume >= 75) return "#ff6b81";
    if (volume >= 45) return "#b388ff";
    return "#64b5f6";
  }

  function overviewLaneDots(moments, width, height, duration) {
    const cache = state.overviewLaneCache;
    if (cache && cache.moments === moments && cache.width === width && cache.height === height && cache.duration === duration) {
      return cache.dots;
    }
    const activeLanes = [];
    const laneSlot = new Map();
    for (const moment of moments) {
      const hits = moment?.[3];
      if (!Array.isArray(hits)) continue;
      for (const hit of hits) {
        const laneIndex = hit?.[0];
        if (typeof laneIndex === "number" && !laneSlot.has(laneIndex)) {
          laneSlot.set(laneIndex, activeLanes.length);
          activeLanes.push(laneIndex);
        }
      }
    }
    const dots = [];
    if (activeLanes.length > 0 && height >= 28) {
      const bandTop = 14;
      const bandHeight = Math.max(10, height - bandTop - 4);
      const laneCount = Math.min(activeLanes.length, 10);
      const step = laneCount > 1 ? bandHeight / (laneCount - 1) : 0;
      const cols = Math.max(1, Math.min(overviewColumns, Math.floor(width / 3)));
      const colWidth = width / cols;
      const drawn = new Set();
      for (const moment of moments) {
        const time = moment?.[0];
        if (!(time >= 0 && time <= duration)) continue;
        const hits = moment?.[3];
        if (!Array.isArray(hits) || !hits.length) continue;
        const col = Math.min(cols - 1, Math.floor((time / duration) * cols));
        const x = (col + 0.5) * colWidth;
        const colour = overviewVolumeHex(moment?.[1] ?? 100);
        for (const hit of hits) {
          const slot = laneSlot.get(hit?.[0]);
          if (slot === undefined) continue;
          const row = laneCount > 1 ? Math.floor((slot * (laneCount - 1)) / Math.max(1, activeLanes.length - 1)) : 0;
          const key = (row << 12) | col;
          if (drawn.has(key)) continue;
          drawn.add(key);
          const y = laneCount > 1 ? bandTop + row * step : bandTop + bandHeight / 2;
          dots.push([x, y, colour]);
        }
      }
    }
    state.overviewLaneCache = { moments, width, height, duration, dots };
    return dots;
  }

  function paintOverviewBase(targetCtx, width, height, duration) {
    targetCtx.fillStyle = slateTimeline.overview;
    targetCtx.fillRect(0, 0, width, height);
    const dots = overviewLaneDots(state.scene?.moments ?? [], width, height, duration);
    const multiLane = dots.length > 0;
    const objY = multiLane ? 7 : height / 2;
    for (const [x, strength] of overviewBins(state.scene.gameplay, width, duration)) {
      targetCtx.fillStyle = "rgba(196,196,196," + strength.toFixed(2) + ")";
      diamond(targetCtx, x, objY, multiLane ? 2.5 : 3.5);
      targetCtx.fill();
    }
    if (multiLane) {
      for (const [x, y, colour] of dots) {
        targetCtx.fillStyle = colour;
        diamond(targetCtx, x, y, 2);
        targetCtx.fill();
      }
    }
  }

  function drawOverview({ ctx: context2d, width, height, scale = 1 }, start, visible) {
    const duration = state.view.duration;
    if (!(duration > 0) || width <= 0) {
      context2d.fillStyle = slateTimeline.overview;
      context2d.fillRect(0, 0, width, height);
      return;
    }
    const middle = height / 2;
    // Cache the static overview background + multi-lane minimap separately from `viewStart` so
    // Centre-mode playback scrolling blits it in one drawImage call instead of redrawing 200+ diamonds.
    let blittedBase = false;
    if (state.view.playing && typeof context2d.drawImage === "function" && height > 0) {
      const baseKey = [state.scene.gameplay, state.scene.moments, width, height, duration, scale];
      if (!state.overviewBaseCache || !sameKey(state.overviewBaseCache.key, baseKey)) {
        try {
          const off = state.overviewBaseCache?.canvas ?? env.document.createElement("canvas");
          const pw = Math.max(1, Math.round(width * scale));
          const ph = Math.max(1, Math.round(height * scale));
          if (off.width !== pw) off.width = pw;
          if (off.height !== ph) off.height = ph;
          const octx = off.getContext?.("2d");
          if (octx && typeof octx.fillRect === "function") {
            octx.setTransform?.(scale, 0, 0, scale, 0, 0);
            paintOverviewBase(octx, width, height, duration);
            state.overviewBaseCache = { key: baseKey, canvas: off };
          }
        } catch {
          state.overviewBaseCache = null;
        }
      }
      if (state.overviewBaseCache?.canvas) {
        context2d.drawImage(state.overviewBaseCache.canvas, 0, 0, width, height);
        blittedBase = true;
      }
    }
    if (!blittedBase) {
      paintOverviewBase(context2d, width, height, duration);
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
    // Problems on this difficulty: 7 px amber diamonds with a 1 px --editor-panel edge, amber so they
    // never read as a red 1/1 tick. Problems under 8 px apart are one diamond; Sound-first hides them.
    context2d.fillStyle = "#fbbf24";
    context2d.strokeStyle = slateTimeline.overview;
    const probKey = [state.soundFirst, state.outputProblemList, state.scene.path, width, duration];
    if (!state.overviewProblemCache || !sameKey(state.overviewProblemCache.key, probKey)) {
      const markers = state.soundFirst ? []
        : visibleWarningMarkers(state.view.outputProblems ?? [], state.scene.path, 0, width / duration, width);
      state.overviewProblemCache = { key: probKey, markers };
    }
    for (const marker of state.overviewProblemCache.markers) {
      diamond(context2d, marker.x, middle, 5);
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
    // Keysounds mode with a track open: the piano roll is drawn on the lanes' canvas instead.
    const roll = ctx.ui()?.keysoundRoll?.() ?? null;
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
      const laneObj = typeof lane === "number" ? state.scene.lanes?.[lane] : null;
      const members = typeof lane === "number" ? (state.drumRowMembers?.get(lane) ?? [lane]) : [];
      const highlighted = members.some(m => {
        const mObj = state.scene.lanes?.[m];
        return m === state.view.activeLane || (mObj != null && state.selectionLaneKeys?.has(mObj.key));
      });
      g.ctx.fillStyle = highlighted ? slateTimeline.activeRow : row % 2 ? slateTimeline.alternateRow : slateTimeline.arrange;
      if (lane === undefined) g.ctx.fillStyle = slateTimeline.arrange;
      else if (typeof lane === "string") g.ctx.fillStyle = slateTimeline.group;
      g.ctx.fillRect(0, y, g.width, rowHeight);
      const tint = laneObj ? ctx.laneTint(laneObj, highlighted) : null;
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
        const drawnPositions = new Set();
        for (const row of state.markedRange.rows) {
          for (const position of state.laneVisualPositions.get(row) ?? []) {
            if (drawnPositions.has(position)) continue;
            drawnPositions.add(position);
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
    for (const problem of state.soundFirst ? [] : state.view.outputProblems ?? []) {
      if (problem.timeline === state.scene.path) problemTimes.add(Math.round(problem.time));
    }
    if (problemTimes.size) {
      g.ctx.fillStyle = "rgba(245,158,11,.12)";
      for (const t of problemTimes) {
        const px = xOf(t);
        if (px >= -8 && px <= g.width + 8) g.ctx.fillRect(px - 4.5, 0, 9, g.height);
      }
    }
    // Binary search into sorted `moments` so off-screen moments are skipped in O(log N) per frame.
    const moments = state.scene.moments;
    const minVisibleTime = start - 8 / zoom;
    const maxVisibleTime = end + 8 / zoom;
    let firstMoment = 0;
    let hiMoment = moments.length;
    while (firstMoment < hiMoment) {
      const mid = (firstMoment + hiMoment) >>> 1;
      if (moments[mid][0] < minVisibleTime) firstMoment = mid + 1;
      else hiMoment = mid;
    }
    // No hit diamonds, guide lines or volume bars in Keysounds mode: skip every moment. That holds
    // with no track open too, when the lanes are covered and the volume lane is out of reach.
    if (roll || ctx.ui()?.mode?.() === "keysounds") firstMoment = moments.length;
    // Batch all visible object vertical guide lines into one Canvas stroke.
    if (state.view.objectLines) {
      let lastObjectPixel = null;
      g.ctx.strokeStyle = "#62d6cf65";
      g.ctx.beginPath();
      for (let mIdx = firstMoment; mIdx < moments.length; mIdx++) {
        const [time, , object] = moments[mIdx];
        if (time > maxVisibleTime) break;
        if (!object) continue;
        const objectPixel = Math.round(xOf(time));
        if (objectPixel !== lastObjectPixel) {
          g.ctx.moveTo(objectPixel + 0.5, 0);
          g.ctx.lineTo(objectPixel + 0.5, g.height);
          lastObjectPixel = objectPixel;
        }
      }
      g.ctx.stroke();
    }
    const volumeLanes = v
      ? new Set(visibleVolumeLanes(state.scene, state.view, state.selectionLaneKeys, state.drumRowMembers, state.drumRowByMember) ?? [])
      : null;
    const activePrimary = state.view.activeLane != null
      ? (state.drumRowByMember?.get(state.view.activeLane) ?? state.view.activeLane)
      : null;
    const activeVolumeMembers = new Set(
      activePrimary != null ? (state.drumRowMembers?.get(activePrimary) ?? [activePrimary]) : [],
    );
    let liveMarquee = null;
    if (state.drag?.marquee && state.drag.lastPoint) {
      const fx = state.drag.firstPoint.x, fy = state.drag.firstPoint.y;
      const lx = state.drag.lastPoint.x, ly = state.drag.lastPoint.y;
      if (Math.hypot(lx - fx, ly - fy) > 3) {
        liveMarquee = {
          minMs: Math.min(Math.max(0, state.drag.firstPoint.ms), state.drag.lastPoint.ms),
          maxMs: Math.max(Math.max(0, state.drag.firstPoint.ms), state.drag.lastPoint.ms),
          loVis: Math.floor(Math.min(fy, ly) / rowHeight),
          hiVis: Math.floor(Math.max(fy, ly) / rowHeight),
          additive: !!state.drag.additive,
        };
      }
    }
    let prevVolumePt = null;
    if (v && volumeLanes.size > 0 && firstMoment > 0) {
      for (let i = firstMoment - 1; i >= 0; i--) {
        const [time, volume, , mHits] = moments[i];
        const vHits = mHits.filter(([row]) => volumeLanes.has(row));
        if (vHits.length > 0) {
          const primary = vHits.find(([row]) => activeVolumeMembers.has(row)) ?? vHits[0];
          const barVolume = primary[3] ?? volume;
          const volH = limit(barVolume, 0, 100) / 100 * (v.height - 3);
          prevVolumePt = [xOf(time), v.height - volH];
          break;
        }
      }
    }
    for (let mIdx = firstMoment; mIdx < moments.length; mIdx++) {
      const [time, volume, , hits] = moments[mIdx];
      if (time > maxVisibleTime) break;
      const x = xOf(time);
      if (x < -8 || x > g.width + 8) continue;
      if (v && volumeLanes.size > 0) {
        const volumeHits = hits.filter(([row]) => volumeLanes.has(row));
        if (volumeHits.length > 0) {
          const primary = volumeHits.find(([row]) => activeVolumeMembers.has(row)) ?? volumeHits[0];
          const barVolume = primary[3] ?? volume;
          const barColour = volumeColour(barVolume);
          const volumeHeight = limit(barVolume, 0, 100) / 100 * (v.height - 3);
          const anySelected = volumeHits.some(([, selected]) => selected);
          const topY = v.height - volumeHeight;
          if (prevVolumePt) {
            v.ctx.strokeStyle = "rgba(255, 255, 255, 0.22)";
            v.ctx.lineWidth = 1.25;
            v.ctx.beginPath();
            v.ctx.moveTo(prevVolumePt[0], prevVolumePt[1]);
            v.ctx.lineTo(x, topY);
            v.ctx.stroke();
            v.ctx.lineWidth = 1;
          }
          prevVolumePt = [x, topY];
          v.ctx.fillStyle = barColour;
          v.ctx.fillRect(x - 2.5, topY, 5, volumeHeight);
          // Bright 5x5 top cap (white when selected), matching 3.1.4 VolumeLane.
          v.ctx.fillStyle = anySelected ? "#ffffff" : barColour;
          v.ctx.fillRect(x - 2.5, Math.max(0, topY - 2), 5, 5);
          // White horizontal ticks for any other shown hit volumes that differ from the bar height.
          for (const [, , , hitVolume] of volumeHits) {
            const effVolume = hitVolume ?? volume;
            if (effVolume !== barVolume) {
              const tickY = v.height - limit(effVolume, 0, 100) / 100 * (v.height - 3);
              v.ctx.fillStyle = "#ffffff";
              v.ctx.fillRect(x - 3, tickY - 0.5, 6, 1.5);
            }
          }
        }
      }
      const hasProblem = problemTimes.has(Math.round(time));
      const seenVisible = new Map();
      for (const [row, selected, listenMuted, hitVolume] of hits) {
        for (const visibleRow of state.laneVisualPositions.get(row) ?? []) {
          const existing = seenVisible.get(visibleRow);
          if (!existing) {
            seenVisible.set(visibleRow, { selected, listenMuted, hitVolume });
          } else {
            existing.selected = existing.selected || selected;
            if (existing.listenMuted && !listenMuted) {
              existing.listenMuted = false;
              existing.hitVolume = hitVolume;
            }
          }
        }
      }
      for (const [visibleRow, { selected, listenMuted, hitVolume }] of seenVisible) {
        const y = (visibleRow + 0.5) * rowHeight - scrollTop;
        if (y < -8 || y > g.height + 8) continue;
        const inLiveBox = liveMarquee !== null
          && time >= liveMarquee.minMs && time <= liveMarquee.maxMs
          && visibleRow >= liveMarquee.loVis && visibleRow <= liveMarquee.hiVis;
        const isSelected = liveMarquee !== null
          ? (liveMarquee.additive ? (selected || inLiveBox) : inLiveBox)
          : selected;
        g.ctx.fillStyle = listenMuted ? "#858585" : volumeColour(hitVolume ?? volume);
        fillHitDiamond(g.ctx, x, y);
        if (isSelected || hasProblem) {
          g.ctx.lineWidth = 2;
          g.ctx.strokeStyle = isSelected ? "#ffffff" : "#e86b72";
          g.ctx.stroke();
          g.ctx.lineWidth = 1;
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
    if (state.drag?.surface === grid && state.drag.lastPoint && !state.cursorAltDraw && state.drag.button !== "right" && ["Select", "Range"].includes(state.view.tool)) {
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
    if (state.pendingDrawHits?.length) {
      for (const hit of state.pendingDrawHits) {
        if (hit.time < minVisibleTime || hit.time > maxVisibleTime) continue;
        const px = xOf(hit.time);
        if (px < -8 || px > g.width + 8) continue;
        const visibleRows = state.laneVisualPositions.get(hit.row) ?? [];
        g.ctx.fillStyle = volumeColour(hit.volume ?? 100);
        for (const visibleRow of visibleRows) {
          const py = (visibleRow + 0.5) * rowHeight - scrollTop;
          if (py < -8 || py > g.height + 8) continue;
          fillHitDiamond(g.ctx, px, py);
        }
      }
    }
    if (state.drawStrokePreview?.steps?.length) {
      const preview = state.drawStrokePreview;
      const visibleRows = state.laneVisualPositions.get(preview.row) ?? [];
      if (visibleRows.length) {
        g.ctx.save();
        g.ctx.globalAlpha = 0.55;
        g.ctx.fillStyle = volumeColour(preview.volume ?? 100);
        g.ctx.strokeStyle = "rgba(255,255,255,0.8)";
        g.ctx.lineWidth = 1.25;
        for (const stepTime of preview.steps) {
          if (stepTime < minVisibleTime || stepTime > maxVisibleTime) continue;
          const px = xOf(stepTime);
          if (px < -8 || px > g.width + 8) continue;
          for (const visibleRow of visibleRows) {
            const py = (visibleRow + 0.5) * rowHeight - scrollTop;
            if (py < -8 || py > g.height + 8) continue;
            fillHitDiamond(g.ctx, px, py);
            g.ctx.stroke();
          }
        }
        g.ctx.restore();
      }
    }
    if (state.drag?.shiftRelative && g.width >= 120 && g.height >= 28) {
      g.ctx.save();
      g.ctx.font = "600 11px Inter, system-ui, sans-serif";
      const label = state.scene?.snapMode === "relative" ? "Grid Snap" : "Relative Grid";
      const tw = g.ctx.measureText(label).width;
      const bw = tw + 16;
      const bx = Math.max(8, g.width - bw - 10);
      const by = 8;
      g.ctx.fillStyle = "rgba(18, 22, 28, 0.88)";
      g.ctx.strokeStyle = "rgba(98, 214, 207, 0.7)";
      g.ctx.lineWidth = 1;
      g.ctx.beginPath();
      g.ctx.roundRect?.(bx, by, bw, 22, 5);
      if (!g.ctx.roundRect) g.ctx.rect(bx, by, bw, 22);
      g.ctx.fill();
      g.ctx.stroke();
      g.ctx.fillStyle = "#e6edf3";
      g.ctx.textBaseline = "middle";
      g.ctx.fillText(label, bx + 8, by + 11);
      g.ctx.restore();
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
    if (roll) {
      // Over whatever the lanes drew: the roll on the same time axis, and its velocity lane.
      g.ctx.fillStyle = slateTimeline.arrange;
      g.ctx.fillRect(0, 0, g.width, g.height);
      paintRoll(g.ctx, g.width, { start, zoom }, roll, roll,
        (context2d, top, bottom) => drawTimingGrid(context2d, start, end, zoom, top, bottom));
      if (v) paintVelocity(v.ctx, v.width, v.height, { start, zoom }, roll);
    }
    r.ctx.fillStyle = slateTimeline.waveform;
    r.ctx.fillRect(0, 0, r.width, r.height);
    const waveformMiddle = (r.height + rulerBandHeight) / 2;
    const waveAmplitude = Math.min(state.waveformHeight, Math.max(0, r.height - rulerBandHeight)) / 2;
    if (state.waveformShown && waveAmplitude > 0) waveform(r.ctx, r.width, waveformMiddle, waveAmplitude, start, r.width / zoom);
    if (state.view?.songLoading && !state.scene.waveform?.length && r.height > rulerBandHeight + 16) {
      r.ctx.save();
      r.ctx.font = "500 11px Inter, system-ui, sans-serif";
      r.ctx.fillStyle = "rgba(230, 238, 242, 0.45)";
      r.ctx.textAlign = "left";
      r.ctx.textBaseline = "middle";
      r.ctx.fillText("Loading song…", 12, waveformMiddle);
      r.ctx.restore();
    }
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
      // The design's loop region: a faint accent band under a crisp 5 px warning-yellow bar.
      r.ctx.fillStyle = slateTimeline.loop;
      r.ctx.fillRect(spanStart, 0, spanWidth, rulerBandHeight - 1);
      r.ctx.fillStyle = slateTimeline.cycle;
      r.ctx.fillRect(spanStart, 0, spanWidth, 5);
    } else {
      const colour = left > right ? "#f0aa70" : slateTimeline.locator;
      r.ctx.fillStyle = colour + (cycling ? "46" : "22");
      r.ctx.fillRect(spanStart, 0, spanWidth, rulerBandHeight - 1);
      r.ctx.fillStyle = colour;
      r.ctx.fillRect(spanStart, 0, spanWidth, 3);
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
    // Keysounds mode: every track's chord conflicts, a ▼ each on the ruler's bottom edge. Passive:
    // the ruler's tooltip names them, and a press there still seeks. Found before the tooltip reads them.
    state.rulerChordMarks = roll ? chordMarks(roll.chords, start, zoom, r.width) : [];
    state.rulerWarningMarkers = state.soundFirst ? []
      : visibleWarningMarkers(state.view.outputProblems ?? [], state.scene.path, start, zoom, r.width);
    ctx.refreshRulerWarningTooltip();
    r.ctx.fillStyle = "#f59e0b";
    for (const marker of state.rulerWarningMarkers) {
      r.ctx.fillRect(marker.x - 3, r.height - 9, 6, 6);
    }
    drawChordMarks(r.ctx, state.rulerChordMarks, rulerBandHeight);
    drawOverview(o, start, g.width / zoom);
    // Only playback reuses the layers, so only then are they copied, and only once the same view has
    // painted twice: while Centre scrolls every frame, a copy would never be used.
    const key = layerKey();
    const repeated = sameKey(state.lastLayerKey, key);
    state.lastLayerKey = key;
    const layers = state.view.playing && repeated ? keepLayers([
      [grid, { name: "g", width: g.width, height: g.height }],
      [ruler, { name: "r", width: r.width, height: r.height }],
      [controller, v && { name: "v", width: v.width, height: v.height }],
      [overview, { name: "o", width: o.width, height: o.height }],
    ]) : null;
    state.staticLayers = layers ? { key, layers, scale: g.scale } : null;
    drawPlayheads(g, r, v, o);
  }

  Object.assign(ctx, {
    prepare, line, sectionBars, drawTimingGrid, waveform, paintSoon,
    anchorPlayhead, playheadPosition, animatePlayback,
    fillHitDiamond, strokeHitDiamond, comboOf, roundedTrack, nodeRadii,
    drawGameplay, overviewBins, diamond, drawOverview, drawLocatorFlags,
    drawBookmarks, paint, paintFrame, drawPlayheads,
  });
}

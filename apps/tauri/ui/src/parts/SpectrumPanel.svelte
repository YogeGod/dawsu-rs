<script>
  import { bridge } from "../lib/bridge.svelte.js";
  import {
    SPECTRUM_BINS, SPECTRUM_FLOOR_DB, applyTilt, binToFreq, freqToNote, freqToX, magmaRgb, xToFreq,
  } from "../lib/spectrum.js";

  const frame = $derived(bridge.spectrum);
  let tilt = $state(4.5);
  let canvasEl = $state(null);
  let hover = $state(null);

  const PEAK_HOLD = new Float32Array(SPECTRUM_BINS).fill(SPECTRUM_FLOOR_DB);
  let waterfallOffscreen = null;
  let rowBuffer = null;

  const pitchReadout = $derived.by(() => {
    if (!frame || !frame.hasPitch || !(frame.peakHz > 0)) return "No clear pitch";
    const info = freqToNote(frame.peakHz);
    if (!info) return `${frame.peakHz.toFixed(1)} Hz`;
    const sign = frame.cents >= 0 ? "+" : "";
    return `${info.name} · ${frame.peakHz.toFixed(1)} Hz (${sign}${frame.cents}c)`;
  });

  const hoverReadout = $derived.by(() => {
    if (!hover) return "";
    const note = hover.note ? `${hover.note.name} · ` : "";
    return `${note}${Math.round(hover.hz)} Hz · ${hover.db.toFixed(1)} dB`;
  });

  function dbToY(db, plotH) {
    const minDb = -90;
    const maxDb = 0;
    const norm = Math.min(1, Math.max(0, (db - minDb) / (maxDb - minDb)));
    return (1 - norm) * (plotH - 6) + 3;
  }

  function paint(spec, tiltDb) {
    if (!canvasEl) return;
    const ctx = canvasEl.getContext?.("2d");
    if (!ctx) return;
    const w = Math.max(240, canvasEl.clientWidth || 320);
    const h = Math.max(120, canvasEl.clientHeight || 180);
    if (canvasEl.width !== w || canvasEl.height !== h) {
      canvasEl.width = w;
      canvasEl.height = h;
    }

    const plotH = Math.floor(h * 0.56);
    const rulerH = 16;
    const fallTop = plotH + rulerH;
    const fallH = Math.max(24, h - fallTop);

    ctx.fillStyle = "#0e0e11";
    ctx.fillRect(0, 0, w, plotH);

    // Calibrated dB & octave grid lines.
    ctx.strokeStyle = "rgba(255,255,255,0.07)";
    ctx.lineWidth = 1;
    ctx.beginPath();
    for (const db of [-12, -24, -48, -72]) {
      const y = Math.round(dbToY(db, plotH)) + 0.5;
      ctx.moveTo(0, y);
      ctx.lineTo(w, y);
    }
    for (const hz of [50, 100, 200, 500, 1000, 2000, 5000, 10000]) {
      const x = Math.round(freqToX(hz, w)) + 0.5;
      ctx.moveTo(x, 0);
      ctx.lineTo(x, plotH);
    }
    ctx.stroke();

    // Piano octave / frequency axis bar.
    ctx.fillStyle = "#16161a";
    ctx.fillRect(0, plotH, w, rulerH);
    ctx.fillStyle = "#9aa0a6";
    ctx.font = "9px system-ui, sans-serif";
    ctx.textBaseline = "middle";
    for (const [hz, label] of [[32.7, "C1"], [65.4, "C2"], [130.8, "C3"], [261.6, "C4"], [523.3, "C5"], [1046.5, "C6"], [2093, "C7"], [4186, "C8"]]) {
      const x = freqToX(hz, w);
      ctx.fillText(label, Math.min(w - 18, x + 2), plotH + rulerH * 0.5);
    }

    const bins = spec?.binsDb;
    if (bins && bins.length >= SPECTRUM_BINS) {
      // Plot peak hold + live magnitude curve.
      ctx.beginPath();
      for (let i = 0; i < SPECTRUM_BINS; i++) {
        const hz = binToFreq(i, SPECTRUM_BINS);
        const val = applyTilt(bins[i], hz, tiltDb);
        PEAK_HOLD[i] = val > PEAK_HOLD[i] ? val : PEAK_HOLD[i] - 0.35;
        const x = freqToX(hz, w);
        const y = dbToY(PEAK_HOLD[i], plotH);
        if (i === 0) ctx.moveTo(x, y);
        else ctx.lineTo(x, y);
      }
      ctx.strokeStyle = "rgba(254, 159, 109, 0.55)";
      ctx.lineWidth = 1;
      ctx.stroke();

      ctx.beginPath();
      for (let i = 0; i < SPECTRUM_BINS; i++) {
        const hz = binToFreq(i, SPECTRUM_BINS);
        const val = applyTilt(bins[i], hz, tiltDb);
        const x = freqToX(hz, w);
        const y = dbToY(val, plotH);
        if (i === 0) ctx.moveTo(x, y);
        else ctx.lineTo(x, y);
      }
      ctx.lineTo(w, plotH);
      ctx.lineTo(0, plotH);
      ctx.closePath();
      ctx.fillStyle = "rgba(90, 200, 250, 0.16)";
      ctx.fill();
      ctx.strokeStyle = "#5ac8fa";
      ctx.lineWidth = 1.4;
      ctx.stroke();

      // Scrolling Magma spectrogram waterfall.
      if (typeof document !== "undefined") {
        if (!waterfallOffscreen || waterfallOffscreen.width !== w || waterfallOffscreen.height !== fallH) {
          waterfallOffscreen = document.createElement("canvas");
          waterfallOffscreen.width = w;
          waterfallOffscreen.height = fallH;
          const wctx = waterfallOffscreen.getContext?.("2d");
          if (wctx) {
            wctx.fillStyle = "#0a0812";
            wctx.fillRect(0, 0, w, fallH);
            rowBuffer = wctx.createImageData?.(w, 1) ?? null;
          }
        }
        const wctx = waterfallOffscreen?.getContext?.("2d");
        if (wctx && rowBuffer) {
          wctx.drawImage(waterfallOffscreen, 0, 0, w, fallH - 1, 0, 1, w, fallH - 1);
          const data = rowBuffer.data;
          for (let x = 0; x < w; x++) {
            const hz = xToFreq(x, w);
            const t = (x / Math.max(1, w - 1)) * (SPECTRUM_BINS - 1);
            const b0 = Math.floor(t);
            const b1 = Math.min(SPECTRUM_BINS - 1, b0 + 1);
            const frac = t - b0;
            const rawDb = bins[b0] * (1 - frac) + bins[b1] * frac;
            const tilted = applyTilt(rawDb, hz, tiltDb);
            const norm = Math.min(1, Math.max(0, (tilted + 90) / 86));
            const [r, g, b] = magmaRgb(norm);
            const px = x * 4;
            data[px] = r;
            data[px + 1] = g;
            data[px + 2] = b;
            data[px + 3] = 255;
          }
          wctx.putImageData(rowBuffer, 0, 0);
          ctx.drawImage(waterfallOffscreen, 0, fallTop);
        }
      }
    } else {
      ctx.fillStyle = "#0a0812";
      ctx.fillRect(0, fallTop, w, fallH);
    }

    if (hover && hover.x >= 0 && hover.x <= w) {
      ctx.strokeStyle = "rgba(255,255,255,0.35)";
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.moveTo(hover.x + 0.5, 0);
      ctx.lineTo(hover.x + 0.5, h);
      ctx.stroke();
    }
  }

  $effect(() => {
    paint(frame, tilt);
  });

  function onpointermove(event) {
    if (!canvasEl) return;
    const rect = canvasEl.getBoundingClientRect();
    const x = Math.max(0, Math.min(rect.width, event.clientX - rect.left));
    const hz = xToFreq(x, rect.width || 1);
    let db = SPECTRUM_FLOOR_DB;
    if (frame?.binsDb?.length >= SPECTRUM_BINS) {
      const idx = Math.min(SPECTRUM_BINS - 1, Math.max(0, Math.round((x / Math.max(1, rect.width)) * (SPECTRUM_BINS - 1))));
      db = applyTilt(frame.binsDb[idx], hz, tilt);
    }
    hover = { x, hz, db, note: freqToNote(hz) };
  }
</script>

<section class="spectrum-panel" aria-label="Spectrum analyzer">
  <header class="spectrum-head">
    <span class="spectrum-title">Spectrum</span>
    <span class="spectrum-pitch">{hoverReadout || pitchReadout}</span>
    <div class="spectrum-tilts" role="group" aria-label="Spectral tilt">
      {#each [0, 3, 4.5] as val (val)}
        <button type="button" class="tilt-btn" aria-pressed={tilt === val}
          onclick={() => (tilt = val)}>{val === 0 ? "Flat" : `+${val}`}</button>
      {/each}
    </div>
    <button type="button" class="spectrum-close" aria-label="Close spectrum"
      onclick={() => bridge.toggleSpectrum()}>×</button>
  </header>
  <canvas class="spectrum-canvas" bind:this={canvasEl}
    onpointermove={onpointermove} onpointerleave={() => (hover = null)}></canvas>
</section>

<style>
  .spectrum-panel {
    flex: 0 0 320px;
    min-width: 240px;
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 8px 10px;
    border-radius: 4px;
    background: var(--editor-panel);
    box-shadow: var(--shadow-card);
    box-sizing: border-box;
    min-height: 0;
  }
  .spectrum-head {
    display: flex;
    align-items: center;
    gap: 6px;
    font: 11px var(--font-ui);
    color: var(--editor-secondary);
  }
  .spectrum-title {
    font-weight: 600;
    color: var(--editor-text);
  }
  .spectrum-pitch {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-variant-numeric: tabular-nums;
    color: var(--editor-readout);
  }
  .spectrum-tilts {
    display: flex;
    gap: 2px;
  }
  .tilt-btn, .spectrum-close {
    border: 0;
    border-radius: 3px;
    padding: 2px 5px;
    min-height: 0;
    background: var(--editor-control);
    color: var(--editor-secondary);
    font: 10px var(--font-ui);
    cursor: pointer;
  }
  .tilt-btn[aria-pressed="true"] {
    background: var(--editor-accent);
    color: var(--editor-ink);
  }
  .spectrum-canvas {
    flex: 1;
    min-height: 0;
    width: 100%;
    border-radius: 3px;
    background: #0e0e11;
    display: block;
  }
</style>

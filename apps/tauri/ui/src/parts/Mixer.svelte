<script>
  import { bridge } from "../lib/bridge.svelte.js";
  import { dbText, gainsAction, hitsoundColour, meterPct, songColour } from "../lib/mixer.js";
  import { laneIcons } from "../lib/lanes.js";
  import Fader from "./Fader.svelte";
  import MixerStrip from "./MixerStrip.svelte";
  import SpectrumPanel from "./SpectrumPanel.svelte";

  // The bottom panel's Mixer tab (v3 F3): the Song bus, the Hitsounds bus, one strip per lane
  // with hits (lanes.js builds them in mixerModel()), and the master output. Every strip has a
  // working fader with dB readout and live audio metering.
  const view = $derived(bridge.view);
  const playhead = $derived(bridge.playhead);
  const scene = $derived(bridge.scene);
  const strips = $derived(bridge.mixer ?? []);
  const disabled = $derived(bridge.disabled);
  const spectrumOpen = $derived(bridge.spectrumOpen);
  const song = $derived(typeof view?.song === "number" ? view.song : null);
  const hitsounds = $derived(typeof view?.hitsounds === "number" ? view.hitsounds : 1);
  const main = $derived(typeof view?.main === "number" ? view.main : null);
  const device = $derived(view?.outputDevice || "");
  const peaks = $derived(playhead?.peaks ?? view?.peaks ?? null);
  const songPeak = $derived(peaks ? (peaks.song ?? 0) : null);
  const hitsoundsPeak = $derived(peaks ? (peaks.hitsounds ?? 0) : null);
  const masterPeak = $derived(peaks ? (peaks.master ?? 0) : null);
  const masterPeakL = $derived(peaks ? (peaks.masterL ?? peaks.master ?? 0) : null);
  const masterPeakR = $derived(peaks ? (peaks.masterR ?? peaks.master ?? 0) : null);
  const masterPctL = $derived(meterPct(masterPeakL));
  const masterPctR = $derived(meterPct(masterPeakR));
  const phaseCorr = $derived(peaks && Number.isFinite(peaks.phaseCorr) ? peaks.phaseCorr : null);
  const phasePct = $derived(phaseCorr != null ? Math.round(((Math.min(1, Math.max(-1, phaseCorr)) + 1) * 0.5) * 100) : 50);
  const songIcon = '<path d="M2 8h1M4.5 5.5v5M7 3v10M9.5 5v6M12 6.5v3M14 8h.5"/>';
  const hitsoundIcon = '<circle cx="8" cy="8" r="5"/><circle cx="8" cy="8" r="2"/>';

  // Estimate per-lane live peak meter from hits firing near the current playhead during playback.
  const lanePeaks = $derived.by(() => {
    if (!peaks) return null;
    const out = new Map();
    if (!playhead?.playing || !scene?.moments?.length || view?.hitsoundsMuted) return out;
    const moments = scene.moments;
    const pos = playhead.position ?? 0;
    const hsGain = typeof view?.hitsounds === "number" ? view.hitsounds : 1;
    const decayMs = 200;
    const windowStart = pos - decayMs;
    let lo = 0;
    let hi = moments.length;
    while (lo < hi) {
      const mid = (lo + hi) >>> 1;
      if (moments[mid][0] < windowStart) lo = mid + 1;
      else hi = mid;
    }
    for (let i = lo; i < moments.length; i++) {
      const [time, vol, , hits] = moments[i];
      const dt = pos - time;
      if (dt < 0) break;
      const env = 1 - dt / decayMs;
      for (const [row, , , hitVol] of hits) {
        const level = ((hitVol ?? vol ?? 100) / 100) * env * hsGain;
        const prev = out.get(row) ?? 0;
        if (level > prev) out.set(row, level);
      }
    }
    return out;
  });

  // Light updates, as the song row and the Audio dialog send them.
  const act = action => bridge.callAction("act", action, false);
  const lane = (...args) => bridge.callAction("laneHeaderAction", ...args);
  // Each fader's last value until the editor answers it, so moving one never sends the other back.
  const pending = {};
  const level = which => async value => {
    pending[which] = value;
    try {
      await act(gainsAction(bridge.view, which, value, pending));
    } finally {
      if (pending[which] === value) delete pending[which];
    }
  };
  const hitsoundLevel = async value => {
    await act({ kind: "hitsound_gain", gain: value });
  };
  const laneLevel = row => async value => {
    await bridge.callAction("act", { kind: "lane_gain", row, gain: value }, true);
  };
</script>

<div class="mixer">
  <div class="strips" role="group" aria-label="Channels">
    <MixerStrip label="Song" name="Song" icon={songIcon} colour={songColour} muted={!!view?.songMuted} solo={!!view?.songSolo}
      muteLabel="Mute song" soloLabel="Solo song" {disabled}
      onmute={() => act({ kind: "song_muted", on: !view?.songMuted })}
      onsolo={() => act({ kind: "song_solo", on: !view?.songSolo })}
      gain={song} min={0} max={1} gainLabel="Song volume" readout={song === null ? null : dbText(song)}
      peak={songPeak} onlevel={level("song")} />
    <MixerStrip label="Hitsounds" name="Hitsounds" icon={hitsoundIcon} colour={hitsoundColour}
      muted={!!view?.hitsoundsMuted} solo={!!view?.hitsoundsSolo}
      muteLabel="Mute all hitsounds" soloLabel="Solo all hitsounds" {disabled}
      onmute={() => act({ kind: "hitsounds_muted", on: !view?.hitsoundsMuted })}
      onsolo={() => act({ kind: "hitsounds_solo", on: !view?.hitsoundsSolo })}
      gain={hitsounds} min={0} max={2} gainLabel="Hitsounds bus volume" readout={dbText(hitsounds)}
      peak={hitsoundsPeak} onlevel={hitsoundLevel} />
    {#each strips as strip (strip.key)}
      <MixerStrip label={strip.name + ", lane " + strip.number} name={strip.name} set={strip.set} number={strip.number}
        icon={laneIcons[strip.icon] ?? laneIcons.file} colour={strip.colour} selected={strip.selected} quiet={strip.quiet}
        muted={strip.muted} solo={strip.solo} muteLabel={"Mute " + strip.name + " (" + strip.set + ")"} soloLabel={"Solo " + strip.name + " (" + strip.set + ")"} {disabled}
        onmute={() => lane("mix", strip.row, "mute")} onsolo={() => lane("mix", strip.row, "solo")}
        onselect={event => lane("select", strip.row, event)}
        gain={strip.gain ?? 1} min={0} max={2} gainLabel={strip.name + " volume"} readout={dbText(strip.gain ?? 1)}
        peak={lanePeaks ? (strip.quiet ? 0 : (lanePeaks.get(strip.row) ?? 0) * (strip.gain ?? 1)) : null}
        onlevel={laneLevel(strip.row)} />
    {/each}
  </div>
  {#if spectrumOpen}
    <SpectrumPanel />
  {/if}
  <section class="master" aria-label="Master output">
    <h3>Master output</h3>
    {#if device}<p class="device" title={device}>{device}</p>{/if}
    <div class="body">
      <Fader gain={main} min={0.2} max={1} label="Master volume" {disabled} peak={masterPeak} peakL={masterPeakL} peakR={masterPeakR} onchange={level("main")} />
      <div class="readouts">
        <span class="caption">Level</span>
        <span class="box level {main === null ? 'off' : ''}">{main === null ? "" : dbText(main)}</span>
        <span class="caption">Peak</span>
        <span class="box peak {masterPeak === null ? 'off' : ''}">{masterPeak != null && masterPeak > 0.001 ? dbText(masterPeak) : ""}</span>
        <div class="peak-bars {masterPeak === null ? 'off' : ''}" aria-hidden="true">
          <div><span class="peak-fill" class:hot={masterPeakL >= 0.5 && masterPeakL < 0.98} class:clip={masterPeakL >= 0.98} style:width={masterPctL + "%"}></span></div>
          <div><span class="peak-fill" class:hot={masterPeakR >= 0.5 && masterPeakR < 0.98} class:clip={masterPeakR >= 0.98} style:width={masterPctR + "%"}></span></div>
        </div>
        <div class="phase-bar {phaseCorr === null ? 'off' : ''}" aria-hidden="true">
          <span class="phase-mid"></span>
          <span class="phase-dot" class:neg={phaseCorr != null && phaseCorr < 0} style:left={phasePct + "%"}></span>
        </div>
        <button type="button" class="spectrum-btn" aria-pressed={spectrumOpen} {disabled}
          onclick={() => bridge.toggleSpectrum()}>Spectrum</button>
      </div>
    </div>
  </section>
</div>

<style>
  .mixer {
    display: flex;
    gap: 12px;
    height: 100%;
    padding: 12px;
    box-sizing: border-box;
    background: var(--editor-canvas);
    overflow: hidden;
  }
  .strips {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: flex-start;
    gap: 2px;
    overflow-x: auto;
    overflow-y: hidden;
    scrollbar-width: thin;
  }
  .master {
    flex: none;
    width: 180px;
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 12px;
    box-sizing: border-box;
    border-radius: 4px;
    background: var(--editor-panel);
    box-shadow: var(--shadow-card);
    overflow: hidden;
  }
  h3 {
    margin: 0;
    font: 600 12px var(--font-ui);
    color: var(--editor-text);
  }
  .device {
    margin: 0;
    font: 12px var(--font-ui);
    color: var(--editor-secondary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .body {
    display: flex;
    gap: 10px;
    margin-top: 6px;
  }
  .readouts {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
    flex: 1;
  }
  .caption {
    font: 11px var(--font-ui);
    color: var(--editor-muted);
  }
  .box {
    height: 18px;
    border-radius: 3px;
    background: rgba(0, 0, 0, 0.45);
    font: 11px var(--font-ui);
    display: grid;
    place-items: center;
    font-variant-numeric: tabular-nums;
    color: var(--editor-readout);
  }
  .box.off {
    opacity: 0.45;
  }
  .peak-bars {
    display: flex;
    flex-direction: column;
    gap: 3px;
    margin-top: 2px;
  }
  .peak-bars div {
    height: 4px;
    border-radius: 2px;
    background: rgba(0, 0, 0, 0.45);
    overflow: hidden;
  }
  .peak-fill {
    display: block;
    height: 100%;
    background: var(--editor-meter, #34c759);
    border-radius: 2px;
  }
  .peak-fill.hot {
    background: var(--editor-warning, #ffd60a);
  }
  .peak-fill.clip {
    background: var(--editor-error, #ff453a);
  }
  .peak-bars.off {
    opacity: 0.45;
  }
  .phase-bar {
    position: relative;
    height: 4px;
    border-radius: 2px;
    background: rgba(0, 0, 0, 0.45);
    margin-top: 2px;
  }
  .phase-bar.off {
    opacity: 0.45;
  }
  .phase-mid {
    position: absolute;
    left: 50%;
    top: 0;
    bottom: 0;
    width: 1px;
    background: rgba(255, 255, 255, 0.16);
  }
  .phase-dot {
    position: absolute;
    top: -1px;
    width: 4px;
    height: 6px;
    margin-left: -2px;
    border-radius: 1px;
    background: var(--editor-meter, #34c759);
  }
  .phase-dot.neg {
    background: var(--editor-error, #ff453a);
  }
  .spectrum-btn {
    margin-top: 4px;
    height: 20px;
    min-height: 20px;
    border: 0;
    border-radius: 3px;
    background: var(--editor-control);
    color: var(--editor-secondary);
    font: 600 10px var(--font-ui);
    cursor: pointer;
  }
  .spectrum-btn[aria-pressed="true"] {
    background: var(--editor-accent);
    color: var(--editor-ink);
  }
  .spectrum-btn:disabled {
    opacity: 0.45;
    cursor: default;
  }
</style>

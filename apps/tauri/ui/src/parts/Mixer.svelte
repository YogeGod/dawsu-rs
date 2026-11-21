<script>
  import { bridge } from "../lib/bridge.svelte.js";
  import { dbText, gainsAction } from "../lib/mixer.js";
  import { laneIcons } from "../lib/lanes.js";
  import Fader from "./Fader.svelte";
  import MixerStrip from "./MixerStrip.svelte";

  // The bottom panel's Mixer tab (v3 F3): the Song strip, one strip per lane with hits (app.js
  // builds them in mixerModel()), and the master output. The song and master gains are the
  // editor's own; lanes have no level of their own yet, so their faders show no cap.
  const view = $derived(bridge.view);
  const strips = $derived(bridge.mixer ?? []);
  const disabled = $derived(bridge.disabled);
  const song = $derived(typeof view?.song === "number" ? view.song : null);
  const main = $derived(typeof view?.main === "number" ? view.main : null);
  const device = $derived(view?.outputDevice || "");
  const songIcon = '<path d="M2 8h1M4.5 5.5v5M7 3v10M9.5 5v6M12 6.5v3M14 8h.5"/>';

  // Light updates, as the song row and the Audio dialog send them.
  const act = action => globalThis.act?.(action, false);
  const lane = (...args) => globalThis.laneHeaderAction?.(...args);
  const level = which => value => act(gainsAction(bridge.view, which, value));
</script>

<div class="mixer">
  <div class="strips" role="group" aria-label="Channels">
    <MixerStrip label="Song" name="Song" icon={songIcon} muted={!!view?.songMuted} solo={!!view?.songSolo}
      muteLabel="Mute song" soloLabel="Solo song" {disabled}
      onmute={() => act({ kind: "song_muted", on: !view?.songMuted })}
      onsolo={() => act({ kind: "song_solo", on: !view?.songSolo })}
      gain={song} min={0} max={1} gainLabel="Song level" readout={song === null ? null : dbText(song)}
      onlevel={level("song")} />
    {#each strips as strip (strip.key)}
      <MixerStrip label={strip.name + ", lane " + strip.number} name={strip.name} set={strip.set} number={strip.number}
        icon={laneIcons[strip.icon] ?? laneIcons.file} colour={strip.colour} selected={strip.selected} quiet={strip.quiet}
        muted={strip.muted} solo={strip.solo} muteLabel={"Mute " + strip.name} soloLabel={"Solo " + strip.name} {disabled}
        onmute={() => lane("mix", strip.row, "mute")} onsolo={() => lane("mix", strip.row, "solo")}
        onselect={event => lane("select", strip.row, event)} />
    {/each}
  </div>
  <section class="master" aria-label="Master output">
    <h3>Master output</h3>
    {#if device}<p class="device" title={device}>{device}</p>{/if}
    <div class="body">
      <Fader gain={main} min={0.2} max={1} label="Master level" {disabled} onchange={level("main")} />
      <div class="readouts">
        <span class="caption">Level</span>
        <span class="box level {main === null ? 'off' : ''}">{main === null ? "" : dbText(main)}</span>
        <span class="caption">Peak</span>
        <span class="box peak off"></span>
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
</style>

<script>
  import { stripShades } from "../lib/mixer.js";
  import Fader from "./Fader.svelte";

  // One v3 mixer strip (§6, 8b footer). Pan, the meter and the level and peak readouts have no
  // backend yet, so they show greyed out and empty; a strip without a gain has a fader without a cap.
  let { label, name, set = "", number = "", icon, colour = null, selected = false, quiet = false,
    muted = false, solo = false, muteLabel, soloLabel, disabled = false, onmute, onsolo, onselect,
    gain = null, min = 0, max = 1, gainLabel, onlevel } = $props();

  const shade = $derived(stripShades(colour));

  // A click outside the strip's controls selects its lane, as on the lane header.
  function click(event) {
    if (disabled || event.target.closest?.("button, [role=slider]")) return;
    onselect?.(event);
  }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
<div class="strip" class:selected class:quiet class:pick={!!onselect} role="group" aria-label={label}
  style:--strip-fill={shade.fill} style:--strip-selected={shade.selected} style:--strip-edge={shade.edge}
  style:--strip-colour={shade.colour} onclick={click}>
  <div class="row">
    <button type="button" class="ms" aria-label={muteLabel} aria-pressed={muted} {disabled} onclick={onmute}>M</button>
    <button type="button" class="ms solo" aria-label={soloLabel} aria-pressed={solo} {disabled} onclick={onsolo}>S</button>
  </div>
  <button type="button" class="box pan off" aria-label="Pan" disabled></button>
  <Fader {gain} {min} {max} label={gainLabel} {disabled} onchange={onlevel} />
  <div class="row">
    <span class="box level off"></span>
    <span class="box peak off"></span>
  </div>
  <div class="foot">
    <span class="set"><svg viewBox="0 0 16 16" aria-hidden="true">{@html icon}</svg>{set}</span>
    <span>{number}</span>
  </div>
  <div class="label">{name}</div>
</div>

<style>
  .strip {
    width: 84px;
    flex: none;
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 5px 5px 0;
    box-sizing: border-box;
    border-radius: 4px;
    overflow: hidden;
    background: var(--strip-fill);
    box-shadow: inset 0 0 0 1px var(--strip-edge), var(--shadow-card);
  }
  .strip.pick {
    cursor: pointer;
  }
  .strip.selected {
    background: var(--strip-selected);
    box-shadow: inset 0 0 0 1px var(--strip-colour), var(--shadow-card);
  }
  .row {
    display: flex;
    gap: 3px;
  }
  .ms {
    flex: 1;
    height: 20px;
    min-height: 20px;
    padding: 0;
    border: 0;
    border-radius: 3px;
    background: var(--ms-idle);
    box-shadow: inset 0 0 0 1px var(--ms-edge);
    color: var(--editor-secondary);
    font: 600 12px var(--font-ui);
    cursor: pointer;
  }
  .ms[aria-pressed="true"] {
    background: var(--mute-on);
    color: var(--editor-ink);
  }
  .ms.solo[aria-pressed="true"] {
    background: var(--solo-on);
  }
  .ms:disabled {
    cursor: default;
  }
  .box {
    flex: 1;
    height: 16px;
    min-height: 16px;
    padding: 0;
    border: 0;
    border-radius: 3px;
    background: rgba(0, 0, 0, 0.45);
    font: 10px var(--font-ui);
    display: grid;
    place-items: center;
    font-variant-numeric: tabular-nums;
    color: var(--editor-readout);
  }
  .box.off {
    opacity: 0.45;
  }
  .pan {
    flex: none;
  }
  .foot {
    height: 14px;
    padding: 0 2px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    font: 600 10px var(--font-ui);
    color: var(--editor-secondary);
    font-variant-numeric: tabular-nums;
  }
  .set {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
  }
  .set svg {
    width: 12px;
    height: 12px;
    flex: none;
    fill: none;
    stroke: var(--strip-colour);
    stroke-width: 1.4;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .label {
    height: 22px;
    flex: none;
    margin: 0 -5px;
    padding: 0 6px;
    background: var(--strip-colour);
    color: var(--editor-ink);
    font: 600 11px var(--font-ui);
    display: flex;
    align-items: center;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .quiet .label {
    background: var(--editor-disabled);
  }
</style>

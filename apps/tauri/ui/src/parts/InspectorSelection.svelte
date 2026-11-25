<script>
  import { untrack } from "svelte";
  import { bridge } from "../lib/bridge.svelte.js";
  import {
    emptyInspector, fieldHold, inspectorHandlers, shownValue, sliderBodyFields, startingAmount, syncChecked, volumeLabel,
  } from "../lib/inspector.js";
  import { additionPills, parseVolumeDisplay, sampleSetPills, volumeSliderPercent } from "../lib/inspectorV2.js";
  import InspectorHead from "./InspectorHead.svelte";

  const on = inspectorHandlers((...args) => globalThis.inspectorAction(...args));
  const model = $derived(bridge.inspector ?? emptyInspector);
  const selection = $derived(model.selection);
  let open = $state(true);

  const parsedVol = $derived(parseVolumeDisplay(selection?.volume));
  let sliderDraft = $state(null);
  const sliderVal = $derived(sliderDraft ?? parsedVol.value);
  const sliderPct = $derived(volumeSliderPercent(sliderVal));
  const setPills = $derived(sampleSetPills(selection));
  const addPills = $derived(additionPills(selection));

  function commitSlider() {
    if (sliderDraft == null) return;
    const target = sliderDraft;
    sliderDraft = null;
    globalThis.inspectorAction?.("volume", target, false);
  }

  // Start and Insert volume keep what is typed until they lose focus (lib/inspector.js).
  let startHeld = $state(null);
  let insertHeld = $state(null);
  const startHold = fieldHold();
  const insertHold = fieldHold();

  let mode = $state("absolute");
  let amount = $state(startingAmount("absolute"));

  // The slider body fields re-seed only when the difficulty or the slider body changes.
  const bodyKey = $derived(selection?.sliderBody?.key ?? null);
  const seed = () => untrack(() => sliderBodyFields(bridge.inspector?.selection?.sliderBody));
  let bodySet = $derived.by(() => (bodyKey, seed().set));
  let bodyWhistle = $derived.by(() => (bodyKey, seed().whistle));
  let bodyWhistleSet = $derived.by(() => (bodyKey, seed().whistleSet));
  let bodyVolume = $derived.by(() => (bodyKey, seed().volume));

  // Re-runs on every update, so a switch the editor refused flips back.
  const showChecked = value => box => syncChecked(box, value());
</script>

<div class="insp insp-first">
  <InspectorHead title="Selection" body="inspector-selection-body" bind:open />
  <div id="inspector-selection-body" hidden={!open}>
    <p class="insp-summary">{selection?.summary ?? "Nothing selected"}</p>
    {#if selection}
      <div class="insp-rows">
        <form class="insp-row" onsubmit={event => on.moveStart(event, event.currentTarget.elements.start, startHold)}>
          <label for="inspector-start" class="insp-label">Start</label>
          <input id="inspector-start" name="start" class="insp-field insp-time" type="text" required spellcheck="false"
            autocomplete="off" placeholder="00:00.000" title="Start, as 01:45.263 or in milliseconds"
            value={shownValue(startHeld, selection.start)} disabled={model.busy}
            onfocus={() => (startHeld = selection.start)}
            onblur={event => { startHeld = null; startHold.release(event.currentTarget, selection?.start); }}>
          <button type="submit" class="insp-small" disabled={model.busy}>Move</button>
        </form>
        <div class="insp-row"><span class="insp-label">Volume</span><output>{selection.volume}</output></div>
        <div class="insp-slider-row">
          <input type="range" class="insp-vol-slider" min="5" max="100" step="1"
            aria-label="Selection volume slider" value={sliderVal} disabled={model.busy}
            style:--pct="{sliderPct}%"
            oninput={event => { sliderDraft = Number(event.currentTarget.value); }}
            onpointerup={commitSlider}
            onchange={commitSlider}>
          <button type="button" class="insp-audition-btn" aria-label="Audition selection" title="Audition selected hits"
            disabled={model.busy} onclick={() => globalThis.inspectorAction?.("auditionSelection")}>▶</button>
        </div>
        <div class="insp-pill-bar" role="group" aria-label="Sample set">
          {#each setPills as pill (pill.id)}
            <button type="button" class="insp-pill" class:insp-pill-on={pill.active} class:insp-pill-mixed={pill.mixed}
              aria-pressed={pill.active} disabled={model.busy || pill.disabled} title={pill.title}
              onclick={() => globalThis.inspectorAction?.("sampleSet", pill.id)}>{pill.label}</button>
          {/each}
        </div>
        <div class="insp-pill-bar" role="group" aria-label="Additions">
          {#each addPills as pill (pill.id)}
            <button type="button" class="insp-pill" class:insp-pill-on={pill.active} class:insp-pill-mixed={pill.mixed}
              aria-pressed={pill.active} disabled={model.busy || pill.disabled} title={pill.title}
              onclick={() => globalThis.inspectorAction?.("addition", pill.id)}>{pill.label}</button>
          {/each}
        </div>
        <div class="insp-row"><span class="insp-label">On object</span><output>{selection.object}</output></div>
      </div>
      <form class="insp-group" aria-label="Change volume" onsubmit={event => on.volume(event, mode, amount)}>
        <p class="insp-caption">Change volume</p>
        <div class="insp-rows">
          <div class="insp-row">
            <label for="inspector-change" class="insp-label">Change</label>
            <select id="inspector-change" value={mode} disabled={model.busy}
              onchange={event => { mode = event.currentTarget.value; amount = startingAmount(mode); }}>
              <option value="absolute">Set all</option><option value="relative">Relative</option>
            </select>
          </div>
          <div class="insp-row">
            <label for="inspector-amount" class="insp-label">{volumeLabel(mode)}</label>
            <span class="insp-value"><input id="inspector-amount" class="insp-field insp-number" type="number"
              min={mode === "relative" ? -100 : 0} max="100" required bind:value={amount} disabled={model.busy}>%</span>
          </div>
          <div class="insp-row">
            <label for="inspector-scope" class="insp-label">Applies to</label>
            <select id="inspector-scope" value={model.scope} disabled={model.busy}
              onchange={event => on.scope(event.currentTarget.value)}>
              <option value="all">All lanes</option><option value="selected">Selected lanes</option>
            </select>
          </div>
        </div>
        <div class="insp-actions">
          <button type="submit" disabled={model.busy}>Apply</button>
          <button type="button" onclick={() => on.nudge(-5)} disabled={model.busy}>−5</button>
          <button type="button" onclick={() => on.nudge(5)} disabled={model.busy}>+5</button>
        </div>
        <p class="insp-caption insp-hint">{model.hint}</p>
      </form>
      <div class="insp-actions">
        <button type="button" onclick={() => on.command("quantizeDialog")} disabled={model.busy}>Quantize…</button>
        <button type="button" onclick={() => on.command("repeat")} disabled={model.busy}>Repeat…</button>
        <button type="button" onclick={() => on.command("duplicate")} disabled={model.busy}>Duplicate</button>
        <button type="button" onclick={() => on.command("delete")} disabled={model.busy}>Delete</button>
      </div>
    {/if}
    <div class="insp-actions">
      <button type="button" onclick={() => on.command("editRange")} disabled={model.busy || !model.range}>Edit range…</button>
      <button type="button" onclick={() => on.command("scaleByVolume")} disabled={!model.doc}>Scale by volume…</button>
    </div>
    {#if selection?.sliderBody}
      <form class="insp-group" onsubmit={event => on.sliderBody(event,
        { set: bodySet, whistleSet: bodyWhistleSet, whistle: bodyWhistle, volume: bodyVolume })}>
        <p class="insp-caption">Slider body sounds</p>
        <div class="insp-rows">
          <label class="insp-row">Slide and tick sample set
            <select aria-label="Slide and tick sample set" bind:value={bodySet}
              disabled={model.busy || !selection.sliderBody.set}>
              <option value="normal">Normal</option><option value="soft">Soft</option><option value="drum">Drum</option>
              <option value="" disabled>Current sample</option>
            </select>
          </label>
          <label class="insp-row">Whistle on slide
            <input type="checkbox" aria-label="Whistle on slide" bind:checked={bodyWhistle} disabled={model.busy}>
          </label>
          <label class="insp-row">Whistle sample set
            <select aria-label="Whistle sample set" bind:value={bodyWhistleSet} disabled={model.busy}>
              <option value="auto">Auto</option><option value="normal">Normal</option><option value="soft">Soft</option>
              <option value="drum">Drum</option>
            </select>
          </label>
          <label class="insp-row">Body and head volume
            <span class="insp-value"><input class="insp-field insp-number" type="number" min="5" max="100"
              aria-label="Body and head volume" bind:value={bodyVolume} disabled={model.busy}>%</span>
          </label>
        </div>
        <div class="insp-actions"><button type="submit" disabled={model.busy}>Apply</button></div>
      </form>
    {/if}
    <div class="insp-rows">
      <label class="insp-row">Insert volume
        <span class="insp-value"><input class="insp-field insp-number" type="number" min="5" max="100"
          aria-label="Insert volume" value={shownValue(insertHeld, model.insertVolume)}
          onfocus={() => (insertHeld = model.insertVolume)}
          onblur={event => { insertHeld = null; insertHold.release(event.currentTarget, model.insertVolume); }}
          onchange={event => on.insert(event, insertHold)}>%</span>
      </label>
      <label class="insp-row">Protect normals
        <input type="checkbox" aria-label="Protect normals" checked={model.protect}
          {@attach showChecked(() => model.protect)} onchange={event => on.toggle("protect", event)}>
      </label>
      <label class="insp-row">Audition
        <input type="checkbox" aria-label="Audition" checked={model.audition}
          {@attach showChecked(() => model.audition)} onchange={event => on.toggle("audition", event)}>
      </label>
    </div>
  </div>
</div>

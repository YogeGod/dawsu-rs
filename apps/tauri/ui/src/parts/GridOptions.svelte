<script>
  import { bridge } from "../lib/bridge.svelte.js";

  const view = $derived(bridge.view);
  const scene = $derived(bridge.scene);
  const disabled = $derived(bridge.disabled);
  const triplets = $derived(bridge.snapGroup?.name === "triplets");
  const send = (...args) => globalThis.popoverAction?.(...args);
</script>

<label class="toggle">
  <input id="object-lines" type="checkbox" checked={!!view?.objectLines} disabled={!scene}
    onchange={event => send("objectLines", event.currentTarget.checked)}>
  Object lines
</label>
<label class="toggle">
  <input id="snap" type="checkbox" checked={!!scene?.snap} disabled={!scene}
    onchange={event => send("snap", { on: event.currentTarget.checked })}>
  Snap
</label>
<select aria-label="Snap mode" id="snap-mode" value={scene?.snapMode ?? "grid"} disabled={!scene}
  onchange={event => send("snap", { mode: event.currentTarget.value })}>
  <option value="grid">Grid</option>
  <option value="relative">Grid relative</option>
  <option value="objects">Objects</option>
</select>
<label class="toggle">
  Beat
  <input id="beats" type="number" min="1" max="64" aria-label="Snap beats" value={scene?.beats ?? 1} disabled={!scene}
    onchange={event => send("snap", { beats: event.currentTarget.value })}>
  /
  <input id="divisor" type="number" min="1" max="64" aria-label="Snap divisor" value={scene?.divisor ?? 4} disabled={!scene}
    onchange={event => send("snap", { divisor: event.currentTarget.value })}>
</label>
<label class="toggle" title="Step through 1/3, 1/6 and 1/12 with the wheel">
  <input id="triplet-snap" type="checkbox" checked={triplets} {disabled}
    onchange={event => send("tripletSnap", event.currentTarget.checked)}>
  Triplets
</label>
<button id="time-signatures" type="button" {disabled} aria-haspopup="dialog"
  onclick={() => send("timeSignatures")}>Time signatures…</button>

<script>
  import { act, bridge } from "../lib/bridge.svelte.js";

  let { laneRow = 0, memberRows = null, laneName = "" } = $props();

  const STEPS = 8;
  const scene = $derived(bridge.scene);
  const view = $derived(bridge.view);
  const rowSet = $derived(new Set(Array.isArray(memberRows) && memberRows.length ? memberRows : [laneRow]));

  const stepGrid = $derived.by(() => {
    const sections = scene?.sections ?? [];
    const pos = view?.position ?? 0;
    let redTime = 0;
    let beatLen = 500;
    for (const sec of sections) {
      if (sec[0] <= pos + 1) {
        redTime = sec[0];
        beatLen = sec[1] > 0 ? sec[1] : 500;
      }
    }
    const divisor = Math.max(1, scene?.divisor ?? 4);
    const stepMs = beatLen / divisor;
    const barMs = beatLen * 4;
    const barStart = redTime + Math.floor(Math.max(0, pos - redTime) / barMs) * barMs;
    const moments = scene?.moments ?? [];
    const out = [];
    for (let i = 0; i < STEPS; i++) {
      const t = Math.round(barStart + i * stepMs);
      let active = false;
      for (const m of moments) {
        if (Math.abs(m[0] - t) <= 4) {
          const hits = m[3] ?? [];
          if (hits.some(h => rowSet.has(h[0]))) {
            active = true;
            break;
          }
        }
      }
      out.push({ idx: i, time: t, active, downbeat: i % divisor === 0 });
    }
    return out;
  });

  async function toggleStep(step) {
    if (bridge.disabled) return;
    await act({
      kind: "toggle_step_hit",
      row: laneRow,
      ms: step.time,
    });
  }
</script>

<div class="step-pads" role="group" aria-label={"Beat step pads for " + laneName}>
  {#each stepGrid as step (step.idx)}
    <button
      type="button"
      class="step-pad"
      class:on={step.active}
      class:downbeat={step.downbeat}
      aria-pressed={step.active}
      aria-label={`${laneName} step ${step.idx + 1} at ${step.time} ms`}
      title={`Step ${step.idx + 1} (${step.time} ms)`}
      onclick={(e) => { e.stopPropagation(); toggleStep(step); }}
    ></button>
  {/each}
</div>

<style>
  .step-pads {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    margin-left: 4px;
    flex-shrink: 0;
  }
  .step-pad {
    width: 9px;
    height: 14px;
    padding: 0;
    border-radius: 2px;
    border: 1px solid var(--editor-border);
    background: var(--editor-canvas);
    cursor: pointer;
  }
  .step-pad.downbeat {
    border-color: var(--editor-muted);
  }
  .step-pad.on {
    background: var(--editor-accent, #4a9eff);
    border-color: var(--editor-accent, #4a9eff);
  }
  .step-pad:hover {
    filter: brightness(1.2);
  }
</style>

<script>
  import { bridge } from "../lib/bridge.svelte.js";
  import { STEP_COUNT, stepsFor, backStep, clampStep, isLastStep, markDone, nextStep, shouldAutoOpen } from "../lib/quickStart.js";

  const storage = () => { try { return globalThis.localStorage; } catch { return null; } };

  // Opens by itself at most once per window, the first time a map is open and the tour is not done.
  let autoChecked = false;
  $effect(() => {
    const view = bridge.view;
    if (autoChecked || bridge.quickStart) return;
    if (!view?.hasDocument) return;
    autoChecked = true;
    if (shouldAutoOpen({ view, storage: storage(), navigator: globalThis.navigator })) bridge.quickStart = 1;
  });

  const step = $derived(clampStep(bridge.quickStart));
  const steps = $derived(stepsFor({ drawKey: bridge.toolDisplayKey("Draw") }));
  const card = $derived(steps[step - 1]);
  const last = $derived(isLastStep(step));

  function finish() {
    markDone(storage());
    bridge.quickStart = 0;
  }

  // A clicked button lets go of focus, so Space plays the song instead of pressing it again.
  const press = run => event => {
    event.currentTarget?.blur?.();
    run();
  };
</script>

{#if bridge.quickStart}
  <div class="quick-start" role="dialog" aria-modal="false" aria-labelledby="quick-start-title">
    <div class="qs-head">
      <span class="qs-count">QUICK START · {step} OF {STEP_COUNT}</span>
      <span class="qs-pills" aria-hidden="true">
        {#each steps as _, index (index)}
          <span class="qs-pill" class:on={index < step}></span>
        {/each}
      </span>
    </div>
    <span id="quick-start-title" class="qs-title">{card.title}</span>
    <span class="qs-body">{card.body}</span>
    <div class="qs-actions">
      <button type="button" class="qs-back" disabled={step === 1} onclick={press(() => { bridge.quickStart = backStep(step); })}>Back</button>
      <button type="button" class="qs-next" onclick={press(() => { if (last) finish(); else bridge.quickStart = nextStep(step); })}>{last ? "Done" : "Next"}</button>
      <span class="qs-gap"></span>
      <button type="button" class="qs-skip" onclick={press(finish)}>Skip tour</button>
    </div>
  </div>
{/if}

<style>
  /* A floating card, no backdrop: the editor around it stays clickable. */
  .quick-start {
    position: fixed;
    top: 112px;
    left: 50%;
    transform: translateX(-50%);
    z-index: 900;
    width: 380px;
    max-width: calc(100vw - 32px);
    box-sizing: border-box;
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 16px;
    background: #292929;
    color: #f5f5f7;
    border: 1px solid #efc16a;
    border-radius: 8px;
    box-shadow: 0 14px 36px rgba(0, 0, 0, .55);
    font-size: 13px;
  }
  .qs-head { display: flex; align-items: center; gap: 8px; }
  .qs-count { flex: 1; font-size: 11px; font-weight: 600; color: #efc16a; white-space: nowrap; }
  .qs-pills { display: flex; gap: 4px; }
  .qs-pill { width: 16px; height: 4px; border-radius: 2px; background: #454545; }
  .qs-pill.on { background: #efc16a; }
  .qs-title { font-size: 15px; font-weight: 600; }
  .qs-body { color: #c4c4c4; line-height: 1.5; }
  .qs-actions { display: flex; align-items: center; gap: 8px; }
  .qs-gap { flex: 1; }
  .qs-actions button {
    height: 30px;
    padding: 0 12px;
    border: 0;
    border-radius: 6px;
    font: inherit;
    font-weight: 500;
    cursor: pointer;
  }
  .qs-back { background: #3c3c3c; color: #f5f5f7; }
  .qs-back:disabled { opacity: .45; cursor: default; }
  .qs-next { background: #efc16a; color: #111111; }
  .qs-skip { background: none; color: #a3a3a3; padding: 0 4px; }
  .qs-skip:hover { color: #f5f5f7; }
</style>

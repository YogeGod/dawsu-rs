<script>
  import { onDestroy } from "svelte";
  import { bridge } from "../lib/bridge.svelte.js";

  const SEEN_KEY = "dawsu.quickStartSeen";
  const steps = [
    {
      title: "1. Song & objects",
      body: "See the song waveform and every hit circle, slider, and combo number above your lanes.",
      selector: ".song-row",
    },
    {
      title: "2. Add & pick samples",
      body: "Browse samples in your mapset or library, preview with a click, and add them as lanes.",
      selector: "#tree-browser-host",
    },
    {
      title: "3. Draw & Snap",
      body: "Left-click or drag to place hits, right-click to erase, and use the Snap slider for beat subdivisions.",
      selector: "#editor-ribbon",
    },
    {
      title: "4. Play & listen",
      body: "Press Space to play or pause, and check the time, BPM, and selected timestamp in the transport bar.",
      selector: "#top-transport",
    },
    {
      title: "5. Save & Export",
      body: "Save updates this difficulty. Export… copies your hitsounds and samples to the whole mapset.",
      selector: "#export-entry",
    },
  ];

  let open = $state(false);
  let stepIndex = $state(0);
  let autoPrompted = false;
  let box = $state(null);

  const hasMap = $derived(Boolean(bridge.scene?.path));
  const step = $derived(steps[stepIndex] ?? steps[0]);

  function readSeen() {
    try {
      return globalThis.localStorage?.getItem(SEEN_KEY) === "1";
    } catch {
      return false;
    }
  }

  function markSeen() {
    try {
      globalThis.localStorage?.setItem(SEEN_KEY, "1");
    } catch {}
  }

  function measureTarget() {
    if (!open || typeof document === "undefined") {
      box = null;
      return;
    }
    const el = document.querySelector(step.selector);
    if (!el || typeof el.getBoundingClientRect !== "function") {
      box = null;
      return;
    }
    const rect = el.getBoundingClientRect();
    if (!rect.width && !rect.height) {
      box = null;
      return;
    }
    box = {
      left: Math.max(4, Math.round(rect.left - 4)),
      top: Math.max(4, Math.round(rect.top - 4)),
      width: Math.round(rect.width + 8),
      height: Math.round(rect.height + 8),
    };
  }

  function startTour() {
    stepIndex = 0;
    open = true;
    measureTarget();
  }

  function closeTour() {
    open = false;
    markSeen();
  }

  function nextStep() {
    if (stepIndex >= steps.length - 1) {
      closeTour();
    } else {
      stepIndex += 1;
      measureTarget();
    }
  }

  function prevStep() {
    if (stepIndex > 0) {
      stepIndex -= 1;
      measureTarget();
    }
  }

  globalThis.openQuickStartTour = startTour;

  $effect(() => {
    if (hasMap && !autoPrompted && !bridge.view?.firstLaunch && !readSeen()) {
      autoPrompted = true;
      startTour();
    }
  });

  $effect(() => {
    if (open) {
      stepIndex;
      measureTarget();
    }
  });

  onDestroy(() => {
    if (globalThis.openQuickStartTour === startTour) {
      delete globalThis.openQuickStartTour;
    }
  });
</script>

{#if open}
  <div class="quick-start-scrim" role="dialog" aria-modal="true" aria-label="Quick Start tour">
    {#if box}
      <div
        class="quick-start-spotlight"
        style:left="{box.left}px"
        style:top="{box.top}px"
        style:width="{box.width}px"
        style:height="{box.height}px"
        aria-hidden="true"
      ></div>
    {/if}
    <div class="quick-start-card">
      <div class="quick-start-step">Step {stepIndex + 1} of {steps.length}</div>
      <h3>{step.title}</h3>
      <p>{step.body}</p>
      <div class="quick-start-actions">
        <button type="button" onclick={closeTour}>Skip</button>
        <span class="spacer"></span>
        {#if stepIndex > 0}
          <button type="button" onclick={prevStep}>Back</button>
        {/if}
        <button type="button" class="primary" onclick={nextStep}>
          {stepIndex === steps.length - 1 ? "Done" : "Next"}
        </button>
      </div>
    </div>
  </div>
{/if}

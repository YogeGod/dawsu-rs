<script>
  import { onDestroy, onMount } from "svelte";
  import { act, bridge } from "../lib/bridge.svelte.js";
  import { invoke } from "../lib/ipc.js";
  import {
    createJobPoller,
    dialogButtons,
    firstUseCopy,
    formatJobStatus,
    formatModelStatus,
    isJobRunning,
    progressPercent,
    removeDownloadedModel,
    watchInBackground,
  } from "../lib/stemSeparation.js";

  // Playback › Separate drums (AI)…: optional on-demand CPU HT-Demucs drum separation.
  // menus.js opens it via bridge modal registry.
  let open = $state(false);
  let busy = $state(false);
  let error = $state("");
  let notice = $state("");
  let status = $state({
    job: { state: "idle" },
    modelReady: false,
    runtimeReady: false,
    modelSizeBytes: null,
    cachedStemPath: null,
    songLoaded: false,
    songName: null,
    activeStemLoaded: false,
    activeStemMonitoring: false,
    downloadBytesNeeded: 0,
    jobMatchesOpenSong: false,
  });

  // One timer: 400 ms while the dialog is open, a slow background watch after it closes mid-job.
  const poller = createJobPoller();
  let lastJobState = "idle";
  const confirm = request => bridge.callAction("confirm", request);
  const statusNow = () => invoke("stem_separation_action", { action: { kind: "status" } });

  const hasDoc = $derived(Boolean(bridge.scene));
  const running = $derived(isJobRunning(status.job));
  const pct = $derived(progressPercent(status.job));
  const jobLine = $derived(formatJobStatus(status.job, status.jobMatchesOpenSong));
  const buttons = $derived(dialogButtons(status));
  const firstUse = $derived(firstUseCopy(status));

  function stopPolling() {
    poller.stop();
  }

  function startPolling() {
    if (poller.mode === "fast") return;
    poller.start("fast", 400, () => {
      void pollStatus();
    });
  }

  function startBackgroundWatch() {
    watchInBackground(poller, {
      poll: statusNow,
      toast: toast => bridge.showToast(toast),
      onFinish: () => act(null),
    });
  }

  function syncPollingFor(next) {
    if (!isJobRunning(next?.job)) return;
    if (open) startPolling();
    else startBackgroundWatch();
  }

  async function pollStatus() {
    try {
      const next = await statusNow();
      if (!next || poller.mode !== "fast") return;
      const prevState = lastJobState;
      const nextState = next.job?.state ?? "idle";
      status = next;
      lastJobState = nextState;

      if (!isJobRunning(next.job)) {
        stopPolling();
        if (nextState === "done" && prevState !== "done") {
          notice = next.jobMatchesOpenSong
            ? "Drum stem loaded. Press T anytime to flip between Song and Drums."
            : "Drum stem cached for another song. Open that song's map to use it.";
          await act(null);
        } else if (nextState === "failed") {
          error = next.job?.message || "Stem separation failed.";
        }
      }
    } catch (err) {
      if (poller.mode !== "fast") return;
      stopPolling();
      error = String(err);
    }
  }

  async function openStemSeparation() {
    if (!hasDoc) return;
    error = "";
    notice = "";
    // Stops a background watch before anything else polls.
    stopPolling();
    open = true;
    busy = true;
    try {
      const next = await statusNow();
      if (next) {
        status = next;
        lastJobState = next.job?.state ?? "idle";
        syncPollingFor(next);
      }
    } catch (err) {
      error = String(err);
    } finally {
      busy = false;
    }
  }

  async function startSeparation(forceRerun) {
    if (busy || running) return;
    error = "";
    notice = "";
    busy = true;
    try {
      const next = await invoke("stem_separation_action", { action: { kind: "start", forceRerun } });
      if (next) {
        status = next;
        lastJobState = next.job?.state ?? "idle";
        if (isJobRunning(next.job)) {
          syncPollingFor(next);
        } else if (next.job?.state === "done") {
          notice = "Drum stem loaded from cache. Press T to flip between Song and Drums.";
          await act(null);
        }
      }
    } catch (err) {
      error = String(err);
    } finally {
      busy = false;
    }
  }

  async function cancelSeparation() {
    if (!running) return;
    error = "";
    try {
      const next = await invoke("stem_separation_action", { action: { kind: "cancel" } });
      if (next) {
        status = next;
        lastJobState = next.job?.state ?? "idle";
      }
      stopPolling();
    } catch (err) {
      error = String(err);
    }
  }

  async function removeModel() {
    if (busy || running || !status.modelReady) return;
    error = "";
    notice = "";
    busy = true;
    try {
      const next = await removeDownloadedModel({ invoke, confirm });
      if (!next) return;
      status = next;
      lastJobState = next.job?.state ?? "idle";
      notice = "Removed the downloaded HT-Demucs model from this computer.";
    } catch (err) {
      error = String(err);
    } finally {
      busy = false;
    }
  }

  function loadOwnStemFile() {
    closeModal();
    bridge.callAction("pickDrumStem");
  }

  // Closing mid-job keeps a slow poll alive only to raise one toast when the job ends.
  function closeModal() {
    if (running) {
      startBackgroundWatch();
    } else {
      stopPolling();
    }
    open = false;
  }

  // A confirm opened above this modal takes the Escape for itself.
  function onKeydown(event) {
    if (open && event.key === "Escape" && bridge.onlyModalOpen("stem-separation")) {
      event.stopPropagation();
      closeModal();
    }
  }

  $effect(() => {
    bridge.setModalOpen("stem-separation", open);
    return () => { bridge.setModalOpen("stem-separation", false); };
  });

  onMount(() => {
    return bridge.registerModal("openStemSeparation", openStemSeparation);
  });

  onDestroy(() => {
    stopPolling();
  });
</script>

<svelte:window onkeydown={onKeydown} />

{#if open}
  <div class="ss-backdrop" role="presentation" onclick={(e) => { if (e.target === e.currentTarget) closeModal(); }}>
    <div class="ss-modal" role="dialog" aria-modal="true" aria-label="Separate drums (AI)">
      <header class="ss-head">
        <span class="ss-title">Separate drums (AI)</span>
        <button type="button" class="ss-close" aria-label="Close" onclick={closeModal}>×</button>
      </header>

      <div class="ss-body">
        <p class="ss-copy">
          Optional: splits the song into a drums-only track locally on your CPU so you can flip
          between Song and Drums with <kbd>T</kbd>.{#if firstUse} {firstUse}.{/if}
        </p>

        <div class="ss-summary">
          <div class="ss-summary-row">
            <span class="ss-label">Model</span>
            <span class="ss-value">{formatModelStatus(status)}</span>
          </div>
          {#if status.songName}
            <div class="ss-summary-row">
              <span class="ss-label">Song</span>
              <span class="ss-value">{status.songName}</span>
            </div>
          {/if}
          {#if status.cachedStemPath}
            <div class="ss-summary-row">
              <span class="ss-label">Cached drums</span>
              <span class="ss-value">Saved in app cache (mapset folder untouched)</span>
            </div>
          {/if}
        </div>

        {#if running || jobLine}
          <div class="ss-progress-box">
            <div class="ss-progress-text">{jobLine}</div>
            {#if running}
              <div class="ss-progress-track" role="progressbar" aria-valuemin="0" aria-valuemax="100" aria-valuenow={pct ?? undefined}>
                {#if pct != null}
                  <div class="ss-progress-fill" style:width={`${pct}%`}></div>
                {:else}
                  <div class="ss-progress-fill ss-progress-indeterminate"></div>
                {/if}
              </div>
            {/if}
          </div>
        {/if}

        <div class="ss-row">
          <button
            type="button"
            class="ss-primary"
            disabled={busy || running || !status.songLoaded}
            onclick={() => startSeparation(buttons.primary.forceRerun)}
          >
            {running ? "Separating…" : buttons.primary.label}
          </button>
          {#if buttons.secondary && !running}
            <button
              type="button"
              disabled={busy || !status.songLoaded}
              onclick={() => startSeparation(buttons.secondary.forceRerun)}
            >
              {buttons.secondary.label}
            </button>
          {/if}
          {#if running}
            <button type="button" onclick={cancelSeparation}>Cancel</button>
          {/if}
        </div>

        <div class="ss-divider ss-secondary-row">
          <button type="button" disabled={busy || running} onclick={loadOwnStemFile}>
            Load your own stem file…
          </button>
          {#if status.modelReady}
            <button
              type="button"
              class="ss-danger"
              disabled={busy || running}
              onclick={removeModel}
            >
              Remove downloaded model
            </button>
          {/if}
        </div>

        {#if error}
          <p class="ss-error" role="alert">{error}</p>
        {:else if notice}
          <p class="ss-notice" role="status">{notice}</p>
        {/if}
      </div>

      <footer class="ss-foot">
        <button type="button" onclick={closeModal}>Close</button>
      </footer>
    </div>
  </div>
{/if}

<style>
  .ss-backdrop {
    position: fixed;
    inset: 0;
    z-index: 62;
    display: grid;
    place-items: center;
    background: rgba(0, 0, 0, 0.68);
  }
  .ss-modal {
    display: flex;
    flex-direction: column;
    width: min(500px, calc(100vw - 40px));
    max-height: min(580px, calc(100vh - 48px));
    border: 1px solid var(--editor-border);
    border-radius: 10px;
    background: var(--editor-panel);
    color: var(--editor-text);
    box-shadow: 0 24px 64px rgba(0, 0, 0, 0.62);
    overflow: hidden;
  }
  .ss-head {
    display: flex;
    align-items: center;
    gap: 12px;
    height: 50px;
    padding: 0 18px;
    border-bottom: 1px solid var(--editor-border);
    background: var(--editor-chrome);
  }
  .ss-title {
    font-size: 14px;
    font-weight: 600;
  }
  .ss-close {
    margin-left: auto;
    width: 28px;
    height: 28px;
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--editor-muted);
    font-size: 18px;
  }
  .ss-body {
    flex: 1;
    padding: 18px 20px;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .ss-copy {
    margin: 0;
    font-size: 12.5px;
    line-height: 1.5;
    color: var(--editor-secondary);
  }
  .ss-summary {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 10px 12px;
    border-radius: 7px;
    background: var(--editor-canvas);
    border: 1px solid var(--editor-border);
    font-size: 12px;
    font-variant-numeric: tabular-nums;
  }
  .ss-summary-row {
    display: flex;
    justify-content: space-between;
    gap: 12px;
  }
  .ss-label {
    color: var(--editor-muted);
  }
  .ss-value {
    color: var(--editor-text);
    text-align: right;
  }
  .ss-progress-box {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px 12px;
    border-radius: 7px;
    background: var(--editor-canvas);
    border: 1px solid var(--editor-border);
  }
  .ss-progress-text {
    font-size: 12px;
    color: var(--editor-text);
    font-variant-numeric: tabular-nums;
  }
  .ss-progress-track {
    height: 6px;
    border-radius: 999px;
    background: rgba(255, 255, 255, 0.08);
    overflow: hidden;
  }
  .ss-progress-fill {
    height: 100%;
    border-radius: 999px;
    background: var(--editor-accent);
    transition: width 160ms ease;
  }
  .ss-progress-indeterminate {
    width: 38%;
    animation: ss-slide 1.2s ease-in-out infinite;
  }
  @keyframes ss-slide {
    0% { transform: translateX(-100%); }
    100% { transform: translateX(300%); }
  }
  .ss-row {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .ss-divider {
    padding-top: 12px;
    border-top: 1px solid var(--editor-border);
  }
  .ss-secondary-row {
    display: flex;
    flex-wrap: wrap;
    justify-content: space-between;
    gap: 8px;
  }
  .ss-primary {
    border-color: var(--editor-accent);
    background: var(--editor-accent);
    color: #0b0e13;
    font-weight: 600;
  }
  .ss-danger {
    color: #ff8e8e;
  }
  .ss-notice {
    margin: 0;
    padding: 8px 10px;
    border-radius: 6px;
    background: rgba(79, 168, 255, 0.12);
    color: var(--editor-text);
    font-size: 12px;
  }
  .ss-error {
    margin: 0;
    padding: 8px 10px;
    border-radius: 6px;
    background: rgba(255, 92, 92, 0.14);
    color: #ff9c9c;
    font-size: 12px;
  }
  .ss-foot {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    padding: 12px 18px;
    border-top: 1px solid var(--editor-border);
    background: var(--editor-chrome);
  }
</style>

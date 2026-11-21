<script>
  import { onMount } from "svelte";

  let toasts = $state([]);
  let nextId = 1;

  function dismiss(id) {
    toasts = toasts.filter(t => t.id !== id);
  }

  function pushToast(input) {
    const id = nextId++;
    const item = {
      id,
      tone: input?.tone || "info",
      title: input?.title || "Notice",
      body: input?.body || "",
      actions: input?.actions || [],
      sticky: Boolean(input?.sticky || input?.actions?.length),
    };
    toasts = [item, ...toasts].slice(0, 3);
    if (!item.sticky) {
      setTimeout(() => dismiss(id), input?.durationMs ?? 5000);
    }
    return id;
  }

  onMount(() => {
    globalThis.showToast = pushToast;
    const events = globalThis.window?.__TAURI__?.event;
    if (events?.listen) {
      events.listen("toast://show", ({ payload }) => pushToast(payload)).catch(() => {});
    }
  });
</script>

{#if toasts.length}
  <div class="toast-stack" role="region" aria-label="Notifications">
    {#each toasts as t (t.id)}
      <div class="toast-card" data-tone={t.tone}>
        <div class="toast-bar" aria-hidden="true"></div>
        <div class="toast-content">
          <div class="toast-head">
            <span class="toast-title">{t.title}</span>
            <button type="button" class="toast-dismiss" aria-label="Dismiss" onclick={() => dismiss(t.id)}>×</button>
          </div>
          {#if t.body}
            <p class="toast-body">{t.body}</p>
          {/if}
          {#if t.actions.length}
            <div class="toast-actions">
              {#each t.actions as act (act.label)}
                <button type="button" class:primary={act.primary} onclick={() => { act.onClick?.(); dismiss(t.id); }}>
                  {act.label}
                </button>
              {/each}
            </div>
          {/if}
        </div>
      </div>
    {/each}
  </div>
{/if}

<style>
  .toast-stack {
    position: fixed;
    right: 16px;
    bottom: 46px;
    z-index: 55;
    display: flex;
    flex-direction: column;
    gap: 10px;
    width: min(380px, calc(100vw - 32px));
    pointer-events: none;
  }
  .toast-card {
    display: flex;
    gap: 12px;
    padding: 12px 14px;
    border: 1px solid var(--editor-border);
    border-radius: 8px;
    background: linear-gradient(180deg, #1e1e21 0%, #151517 100%);
    color: var(--editor-text);
    box-shadow: 0 14px 36px rgba(0, 0, 0, 0.52);
    pointer-events: auto;
  }
  .toast-bar {
    width: 3px;
    border-radius: 2px;
    background: var(--editor-accent);
    flex: none;
  }
  .toast-card[data-tone="warning"] .toast-bar { background: var(--editor-warning); }
  .toast-card[data-tone="success"] .toast-bar { background: var(--editor-play); }
  .toast-card[data-tone="error"] .toast-bar { background: var(--editor-error); }
  .toast-content {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 5px;
  }
  .toast-head {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .toast-title {
    flex: 1;
    font-size: 12px;
    font-weight: 600;
  }
  .toast-dismiss {
    width: 20px;
    height: 20px;
    min-height: 20px;
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--editor-muted);
    font-size: 15px;
  }
  .toast-body {
    margin: 0;
    font-size: 12px;
    line-height: 1.4;
    color: var(--editor-secondary);
  }
  .toast-actions {
    display: flex;
    gap: 8px;
    margin-top: 4px;
  }
  .toast-actions button.primary {
    background: var(--editor-accent);
    color: var(--editor-accentink);
    font-weight: 600;
  }
</style>

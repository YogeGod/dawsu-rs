<script>
  import { onMount, tick } from "svelte";
  import { bridge } from "../lib/bridge.svelte.js";
  import { makeToast, pauseToast, pushToast as addToast, resumeToast } from "../lib/toasts.js";

  let toasts = $state([]);
  let nextId = 1;
  let hovered = false;
  let focused = false;
  let stackEl = null;
  let destroyed = false;
  const timers = new Map();
  const errors = $derived(toasts.filter(t => t.tone === "error"));
  const others = $derived(toasts.filter(t => t.tone !== "error"));
  const paused = () => hovered || focused;
  const now = () => globalThis.performance?.now() ?? Date.now();

  function clearTimer(id) {
    clearTimeout(timers.get(id));
    timers.delete(id);
  }

  function schedule(toast) {
    clearTimer(toast.id);
    if (toast.sticky || paused()) return;
    timers.set(toast.id, setTimeout(() => dismiss(toast.id), toast.remainingMs));
  }

  function dismiss(id) {
    clearTimer(id);
    toasts = toasts.filter(t => t.id !== id);
    // A focused button that is removed may never send focusout, and an empty stack stops
    // taking the pointer, so a pointerleave may never come either.
    setPaused(() => {
      focused = false;
      if (!toasts.length) hovered = false;
    });
  }

  function pushToast(input) {
    const id = nextId++;
    let item = makeToast(input, id);
    if (!paused()) item = resumeToast(item, now());
    const prevIds = toasts.map(t => t.id);
    toasts = addToast(toasts, item);
    const evicted = prevIds.some(prevId => !toasts.some(t => t.id === prevId));
    for (const key of [...timers.keys()]) if (!toasts.some(t => t.id === key)) clearTimer(key);
    if (toasts.some(t => t.id === id)) schedule(item);
    if (focused && evicted) {
      tick().then(() => {
        const active = globalThis.document?.activeElement;
        if (!destroyed && focused && (!active || !stackEl?.contains(active))) {
          setPaused(() => { focused = false; });
        }
      });
    }
    return id;
  }

  function pause() {
    const at = now();
    for (const key of [...timers.keys()]) clearTimer(key);
    toasts = toasts.map(t => pauseToast(t, at));
  }

  function resume() {
    const at = now();
    toasts = toasts.map(t => resumeToast(t, at));
    for (const t of toasts) schedule(t);
  }

  function setPaused(update) {
    const was = paused();
    update();
    if (was !== paused()) (paused() ? pause : resume)();
  }

  const onPointerEnter = () => setPaused(() => { hovered = true; });
  const onPointerLeave = () => setPaused(() => { hovered = false; });
  const onFocusIn = () => setPaused(() => { focused = true; });
  function onFocusOut(event) {
    if (event.currentTarget.contains(event.relatedTarget)) return;
    setPaused(() => { focused = false; });
  }

  onMount(() => {
    destroyed = false;
    const disposeToast = bridge.registerToast(pushToast);
    const events = globalThis.window?.__TAURI__?.event;
    let unlisten = null;
    if (events?.listen) {
      Promise.resolve(events.listen("toast://show", ({ payload }) => {
        if (!destroyed) pushToast(payload);
      }))
        .then((stop) => {
          if (destroyed) stop?.();
          else unlisten = stop;
        })
        .catch(() => {});
    }
    return () => {
      destroyed = true;
      disposeToast();
      unlisten?.();
      for (const key of [...timers.keys()]) clearTimer(key);
    };
  });
</script>

{#snippet card(t)}
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
{/snippet}

<div
  bind:this={stackEl}
  class="toast-stack"
  class:has-toasts={toasts.length > 0}
  role="region"
  aria-label="Notifications"
  onpointerenter={onPointerEnter}
  onpointerleave={onPointerLeave}
  onfocusin={onFocusIn}
  onfocusout={onFocusOut}
>
  <div class="toast-group" role="alert">
    {#each errors as t (t.id)}{@render card(t)}{/each}
  </div>
  <div class="toast-group" class:spaced={errors.length > 0 && others.length > 0} role="status" aria-live="polite">
    {#each others as t (t.id)}{@render card(t)}{/each}
  </div>
</div>

<style>
  .toast-stack {
    position: fixed;
    right: 16px;
    bottom: 46px;
    z-index: var(--z-toast);
    display: flex;
    flex-direction: column;
    width: min(380px, calc(100vw - 32px));
    max-height: calc(100vh - 92px);
    overflow-y: auto;
    pointer-events: none;
  }
  /* Only a stack with cards takes the pointer, so its scrollbar and the gaps between cards stay live. */
  .toast-stack.has-toasts {
    pointer-events: auto;
  }
  .toast-group {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .toast-group.spaced {
    margin-top: 10px;
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

// Pure toast list helpers for ToastStack.svelte: building, capping and pausable timing.

export const DEFAULT_TOAST_MS = 5000;

// Errors and toasts with actions stay until the user dismisses them.
export function makeToast(input, id) {
  const actions = input?.actions || [];
  const tone = input?.tone || "info";
  const sticky = Boolean(input?.sticky || actions.length || tone === "error");
  return {
    id,
    tone,
    title: input?.title || "Notice",
    body: input?.body || "",
    actions,
    sticky,
    remainingMs: sticky ? null : (input?.durationMs ?? DEFAULT_TOAST_MS),
    startedAt: null,
  };
}

// Newest first. Over the cap the oldest non-sticky toast goes; sticky toasts are never dropped.
export function pushToast(list, item, cap = 3) {
  const next = [item, ...list];
  while (next.length > cap) {
    let oldest = -1;
    for (let at = next.length - 1; at >= 0; at -= 1) {
      if (!next[at].sticky) { oldest = at; break; }
    }
    if (oldest < 0) break;
    next.splice(oldest, 1);
  }
  return next;
}

// Starts (or restarts) a timed toast's clock at `now`.
export function resumeToast(toast, now) {
  if (toast.sticky) return toast;
  return { ...toast, startedAt: now };
}

// Stops a timed toast's clock, keeping only the time it has left.
export function pauseToast(toast, now) {
  if (toast.sticky || toast.startedAt == null) return toast;
  return { ...toast, remainingMs: Math.max(0, toast.remainingMs - (now - toast.startedAt)), startedAt: null };
}

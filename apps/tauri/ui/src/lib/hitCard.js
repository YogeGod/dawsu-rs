// states.md Tooltips: a tooltip waits 500 ms, or shows at once when the last one closed under 1 s ago.
export function tooltipDelay(now, lastHidden) {
  return lastHidden !== null && now - lastHidden < 1000 ? 0 : 500;
}

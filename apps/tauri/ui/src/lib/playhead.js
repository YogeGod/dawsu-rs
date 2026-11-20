// The Playhead group (v3 §3.5). Follow and Return are editor settings; Centre lives in the window only.
export const playheadActions = view => ({
  follow: { kind: "follow", on: !view?.follow },
  return: { kind: "return_on_stop", on: !view?.returnOnStop },
});

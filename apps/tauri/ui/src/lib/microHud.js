// Pure positioning and mode-formatting helpers for the floating Inline Quick-Edit Micro-HUD.

export const MICRO_HUD_WIDTH = 248;
export const MICRO_HUD_HEIGHT = 44;

/**
 * Clamps the Micro-HUD anchor beside a selected hit/note so it stays inside the viewport.
 */
export function clampMicroHud(anchorX, anchorY, viewportWidth, viewportHeight) {
  const margin = 10;
  const maxLeft = Math.max(margin, viewportWidth - MICRO_HUD_WIDTH - margin);
  const left = Math.round(Math.max(margin, Math.min(maxLeft, anchorX - MICRO_HUD_WIDTH / 2)));
  const above = anchorY - MICRO_HUD_HEIGHT - 14;
  const below = anchorY + 18;
  const top = Math.round(above >= margin ? above : Math.min(viewportHeight - MICRO_HUD_HEIGHT - margin, below));
  return { left, top };
}

/**
 * Returns the compact mode label and chord-policy options for the Micro-HUD.
 */
export function microHudMode(mode) {
  if (mode === "drums") return "drums";
  if (mode === "keysounds") return "keysounds";
  return "hitsounds";
}

export const KEYSOUND_CHORD_POLICIES = [
  { id: "flatten", label: "Flatten" },
  { id: "top_only", label: "Top only" },
];

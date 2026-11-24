// Pure hover-hint helpers for the editor's bottom status bar.
// Maps (zone, activeTool) -> compact [{ keys, action }] gesture hints in plain osu! mapper English.

const TIMELINE_HINTS = {
  Select: [
    { keys: "Drag", action: "Box select" },
    { keys: "Drag hit", action: "Move" },
    { keys: "Alt+Click", action: "Draw hit" },
    { keys: "Del", action: "Delete" },
  ],
  Draw: [
    { keys: "Click", action: "Place hit" },
    { keys: "Right-click", action: "Delete" },
    { keys: "Alt+Wheel", action: "Zoom" },
  ],
  Erase: [
    { keys: "Click / Drag", action: "Erase hits" },
    { keys: "Alt+Click", action: "Draw hit" },
    { keys: "Alt+Wheel", action: "Zoom" },
  ],
  Mute: [
    { keys: "Click / Drag", action: "Mute hits" },
    { keys: "Alt+Click", action: "Draw hit" },
    { keys: "Alt+Wheel", action: "Zoom" },
  ],
  Grab: [
    { keys: "Drag", action: "Pan timeline" },
    { keys: "Alt+Wheel", action: "Zoom" },
    { keys: "Wheel", action: "Seek" },
  ],
  Range: [
    { keys: "Drag", action: "Mark range" },
    { keys: "Ctrl+C / V", action: "Copy / Paste" },
    { keys: "Del", action: "Clear range" },
  ],
  Ramp: [
    { keys: "Drag", action: "Ramp volume" },
    { keys: "Right-click", action: "Delete" },
    { keys: "Alt+Wheel", action: "Zoom" },
  ],
};

const ZONE_HINTS = {
  ruler: [
    { keys: "Drag", action: "Set cycle loop" },
    { keys: "Ctrl+Click", action: "L locator" },
    { keys: "Alt+Click", action: "R locator" },
    { keys: "Wheel", action: "Zoom" },
  ],
  volume: [
    { keys: "Drag", action: "Adjust" },
    { keys: "Alt+Drag", action: "Paint" },
    { keys: "Ctrl+Drag", action: "Spread" },
    { keys: "Shift+Drag", action: "Ramp" },
  ],
  lanes: [
    { keys: "Click", action: "Audition & select" },
    { keys: "M", action: "Mute" },
    { keys: "S", action: "Solo" },
    { keys: "Right-click", action: "Lane menu" },
  ],
  mixer: [
    { keys: "Drag fader", action: "Lane volume" },
    { keys: "M / S", action: "Mute / Solo" },
    { keys: "Click", action: "Select lane" },
  ],
  browser: [
    { keys: "Click", action: "Preview sample" },
    { keys: "Double-click", action: "Use sample" },
    { keys: "Right-click", action: "Options" },
  ],
  inspector: [
    { keys: "Click", action: "Edit properties" },
    { keys: "Enter", action: "Apply value" },
  ],
  overview: [
    { keys: "Click / Drag", action: "Jump & pan song" },
    { keys: "Wheel", action: "Zoom" },
  ],
};

export function hintsFor(zone, tool) {
  if (!zone || typeof zone !== "string") return [];
  if (zone === "timeline") {
    return TIMELINE_HINTS[tool] ?? TIMELINE_HINTS.Select;
  }
  return ZONE_HINTS[zone] ?? [];
}

export function hintZoneFromTarget(target) {
  const host = target?.closest?.("[data-hint-zone]");
  if (host?.dataset?.hintZone) return host.dataset.hintZone;
  if (target?.closest?.(".mixer-host")) return "mixer";
  return null;
}

export function hasPriorityStatus(notice, view) {
  return Boolean(notice || view?.fault || view?.busy);
}

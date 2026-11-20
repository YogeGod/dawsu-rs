// The editor's seven tools, in the order of toolIcons in app.js (the right-click picker lists all).
export const TOOLS = ["Select", "Draw", "Erase", "Mute", "Grab", "Range", "Ramp"];
const pills = TOOLS.slice(0, 3);

// Mute, Grab, Range and Ramp live in the picker; the current one gets a pill so it stays visible.
export const toolPills = tool => TOOLS.includes(tool) && !pills.includes(tool) ? [...pills, tool] : pills;

export const toolAction = name => ({ kind: "tool", name });
export const auditionAction = view => ({ kind: "audition", on: !view.audition });

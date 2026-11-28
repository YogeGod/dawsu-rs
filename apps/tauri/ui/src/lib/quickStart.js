// The first-run Quick start tour (design f14): five cards, Back / Next / Skip, done kept in
// localStorage on this computer only.

export const STORAGE_KEY = "dawsu.quickStartDone";

// The cards, with the Draw key of the current preset and shortcuts (null when Draw has none).
export const stepsFor = ({ drawKey = null } = {}) => [
  {
    title: "Pick a sound for each row",
    body: "The Browser on the left holds your samples. Click one to hear it, then Use in active lane to give that row its sound.",
  },
  {
    title: "Draw a hit on a circle",
    body: `Each row plays one sound. With ${drawKey ? `Draw (${drawKey})` : "Draw"}, click a row under a circle and the hit snaps onto it. Press Space to hear it with the song.`,
  },
  {
    title: "Set how loud each hit is",
    body: "Open Volume and Mixer under the timeline. Drag a bar up or down to change a hit's volume; hits at one moment move together.",
  },
  {
    title: "Check the difficulty",
    body: "The Difficulty card in the Inspector marks problems in this difficulty. Generate…, New… and Copy… fill other difficulties from it.",
  },
  {
    title: "Save and export",
    body: "Save writes the hitsounds into the .osu file. Export… copies your hitsounds into the other difficulties.",
  },
];

export const STEP_COUNT = stepsFor().length;

// Steps are 1-based, as the header shows them.
export const clampStep = step => Math.min(STEP_COUNT, Math.max(1, Math.trunc(Number(step)) || 1));
export const nextStep = step => clampStep(step + 1);
export const backStep = step => clampStep(step - 1);
export const isLastStep = step => clampStep(step) === STEP_COUNT;

export function isDone(storage) {
  try {
    return storage?.getItem?.(STORAGE_KEY) === "1";
  } catch {
    return false;
  }
}

export function markDone(storage) {
  try {
    storage?.setItem?.(STORAGE_KEY, "1");
  } catch {}
}

// Open by itself once, when a map is open in the editor and nothing else asks first: not over the
// Start Screen or the first-launch preset chooser, and never under automation (navigator.webdriver).
export function shouldAutoOpen({ view, storage, navigator } = {}) {
  if (navigator?.webdriver === true) return false;
  if (!view?.hasDocument || view.showHome || view.firstLaunch) return false;
  return !isDone(storage);
}

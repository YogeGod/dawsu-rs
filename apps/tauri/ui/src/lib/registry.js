// Pure-JS action, modal, toast & chrome registry shared between the editor runtime and bridge.svelte.js.
// Kept free of Svelte runes so plain Node integration tests can import editor modules directly.

export const actions = {};
export const modals = {};

const openModals = new Set();
let shortcutCapturing = false;
let toastHandler = null;
let paletteMenuApi = null;
let timelineThemeTarget = null;

export function registerAction(name, fn) {
  actions[name] = fn;
  return () => {
    if (actions[name] === fn) delete actions[name];
  };
}

export function callAction(name, ...args) {
  const fn = actions[name] ?? globalThis[name];
  return typeof fn === "function" ? fn(...args) : undefined;
}

export function registerModal(name, fn) {
  modals[name] = fn;
  return () => {
    if (modals[name] === fn) delete modals[name];
  };
}

export function openModal(name, ...args) {
  const fn = modals[name] ?? globalThis[name];
  return typeof fn === "function" ? fn(...args) : undefined;
}

export function setModalOpen(id, open) {
  if (open) openModals.add(id);
  else openModals.delete(id);
}

export function hasOpenModal() {
  return openModals.size > 0;
}

// True when no modal other than `id` is open.
export function onlyModalOpen(id) {
  for (const open of openModals) if (open !== id) return false;
  return true;
}

export function setShortcutCapturing(on) {
  shortcutCapturing = Boolean(on);
}

export function isShortcutCapturing() {
  return shortcutCapturing;
}

export function registerToast(fn) {
  toastHandler = fn;
  return () => {
    if (toastHandler === fn) toastHandler = null;
  };
}

export function showToast(toast) {
  const fn = toastHandler ?? globalThis.showToast;
  return typeof fn === "function" ? fn(toast) : undefined;
}

export function registerPaletteMenu(api) {
  paletteMenuApi = api ?? null;
}

export function getPaletteMenu() {
  return paletteMenuApi ?? globalThis.dawsuPaletteMenu ?? null;
}

export function registerTimelineTheme(target) {
  timelineThemeTarget = target ?? null;
}

export function getTimelineTheme() {
  return timelineThemeTarget ?? globalThis.slateTimeline ?? null;
}

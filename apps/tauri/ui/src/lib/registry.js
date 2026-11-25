// Pure-JS action & modal registry shared between the editor runtime and bridge.svelte.js.
// Kept free of Svelte runes so plain Node integration tests can import editor modules directly.

export const actions = {};
export const modals = {};

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

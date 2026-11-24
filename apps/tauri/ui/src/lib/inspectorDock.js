// Inspector section reordering and floating-panel detachment helpers.
// Wired once from InspectorHead.svelte so every Inspector section gets drag-to-reorder
// and drag-onto-canvas floating automatically.

export const ORDER_STORAGE_KEY = "dawsu.inspectorOrder";
export const DRAG_THRESHOLD_PX = 6;
export const DETACH_MARGIN_PX = 24;

let topFloatingZ = 40;

/**
 * Merge a saved section order with the currently available section IDs.
 * Keeps valid saved IDs in order, drops unknown/duplicate IDs, and appends any new IDs.
 */
export function normalizeOrder(savedOrder, availableIds) {
  const available = Array.isArray(availableIds) ? [...new Set(availableIds.filter(Boolean))] : [];
  if (!Array.isArray(savedOrder) || !savedOrder.length) return available;
  const availableSet = new Set(available);
  const result = [];
  const seen = new Set();
  for (const id of savedOrder) {
    if (typeof id === "string" && availableSet.has(id) && !seen.has(id)) {
      seen.add(id);
      result.push(id);
    }
  }
  for (const id of available) {
    if (!seen.has(id)) {
      seen.add(id);
      result.push(id);
    }
  }
  return result;
}

/**
 * Return a new array with draggedId moved to targetIndex (clamped to bounds).
 */
export function reorderIds(ids, draggedId, targetIndex) {
  const list = Array.isArray(ids) ? [...ids] : [];
  const from = list.indexOf(draggedId);
  if (from < 0) return list;
  const to = Math.max(0, Math.min(list.length - 1, Math.round(targetIndex)));
  if (from === to) return list;
  const [item] = list.splice(from, 1);
  list.splice(to, 0, item);
  return list;
}

/**
 * Given vertical rects of visible docked sections and the pointer Y, return the target index.
 */
export function computeDropIndex(rects, pointerY, currentIndex) {
  if (!Array.isArray(rects) || rects.length <= 1) return Math.max(0, currentIndex);
  for (let i = 0; i < rects.length; i++) {
    const mid = (rects[i].top + rects[i].bottom) / 2;
    if (pointerY < mid) return i;
  }
  return rects.length - 1;
}

/**
 * True when the pointer has been dragged horizontally outside the Inspector sidebar.
 */
export function shouldDetach(pointerX, sidebarRect, margin = DETACH_MARGIN_PX) {
  if (!sidebarRect || !(sidebarRect.width > 0)) return false;
  return pointerX < sidebarRect.left - margin || pointerX > sidebarRect.right + margin;
}

/**
 * Clamp a floating panel's top-left position so its header stays reachable in the viewport.
 */
export function clampFloatingPosition(x, y, width, height, viewportWidth, viewportHeight) {
  const pad = 8;
  const vw = Number.isFinite(viewportWidth) && viewportWidth > 0 ? viewportWidth : 1280;
  const vh = Number.isFinite(viewportHeight) && viewportHeight > 0 ? viewportHeight : 720;
  const w = Math.min(Math.max(120, Number(width) || 260), Math.max(120, vw - pad * 2));
  const maxLeft = Math.max(pad, vw - w - pad);
  const maxTop = Math.max(pad, vh - 40);
  return {
    left: Math.round(Math.max(pad, Math.min(maxLeft, Number(x) || pad))),
    top: Math.round(Math.max(pad, Math.min(maxTop, Number(y) || pad))),
  };
}

export function readSavedOrder(storage = globalThis.localStorage) {
  try {
    const raw = storage?.getItem?.(ORDER_STORAGE_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    return Array.isArray(parsed) ? parsed.filter(item => typeof item === "string" && item.length > 0) : [];
  } catch {
    return [];
  }
}

export function writeSavedOrder(order, storage = globalThis.localStorage) {
  try {
    if (!Array.isArray(order)) return;
    storage?.setItem?.(ORDER_STORAGE_KEY, JSON.stringify(order));
  } catch {
    // Ignore storage quota or private-mode errors.
  }
}

/**
 * Resolve the DOM nodes for an Inspector section from its .insp-head button.
 */
export function findInspectorNodes(headEl) {
  if (!headEl) return null;
  const panel = headEl.closest(".insp");
  const sidebar = headEl.closest("#editor-inspector, #inspector");
  const host = headEl.closest("section.inspector-part") ?? headEl.closest("[data-ui-mount]");
  const mount = host?.matches?.("[data-ui-mount]") ? host : host?.querySelector?.("[data-ui-mount]");
  const id = mount?.dataset?.uiMount || headEl.getAttribute("aria-controls") || "";
  return { panel, sidebar, host, id };
}

/**
 * List all section host elements currently inside the Inspector sidebar.
 */
export function getSidebarSections(sidebar) {
  if (!sidebar) return [];
  const children = [...sidebar.children].filter(
    el => el.matches?.("section.inspector-part") || el.matches?.("[data-ui-mount]"),
  );
  return children.map(el => {
    const mount = el.matches?.("[data-ui-mount]") ? el : el.querySelector?.("[data-ui-mount]");
    const head = el.querySelector?.(".insp-head");
    const id = mount?.dataset?.uiMount || head?.getAttribute?.("aria-controls") || "";
    return { el, id };
  }).filter(item => item.id);
}

/**
 * Restore the persisted section order inside the Inspector sidebar.
 */
export function applySavedOrder(sidebar, storage = globalThis.localStorage) {
  const sections = getSidebarSections(sidebar);
  if (sections.length <= 1) return false;
  const currentIds = sections.map(s => s.id);
  const desiredIds = normalizeOrder(readSavedOrder(storage), currentIds);
  if (desiredIds.every((id, index) => id === currentIds[index])) return false;

  const byId = new Map(sections.map(s => [s.id, s.el]));
  for (const id of desiredIds) {
    const el = byId.get(id);
    if (el) sidebar.appendChild(el);
  }
  return true;
}

/**
 * Persist the current DOM order of Inspector sections.
 */
export function saveCurrentOrder(sidebar, storage = globalThis.localStorage) {
  const ids = getSidebarSections(sidebar).map(s => s.id);
  if (ids.length) writeSavedOrder(ids, storage);
  return ids;
}

/**
 * Move a section host element to targetIndex among the sidebar's section hosts.
 */
export function moveHostToIndex(sidebar, hostEl, targetIndex) {
  const sections = getSidebarSections(sidebar);
  const currentIndex = sections.findIndex(s => s.el === hostEl);
  if (currentIndex < 0) return false;
  const clamped = Math.max(0, Math.min(sections.length - 1, targetIndex));
  if (clamped === currentIndex) return false;

  const withoutDragged = sections.filter(s => s.el !== hostEl);
  const beforeEl = withoutDragged[clamped]?.el ?? null;
  if (beforeEl) sidebar.insertBefore(hostEl, beforeEl);
  else sidebar.appendChild(hostEl);
  return true;
}

/**
 * Bring a floating Inspector panel to the front of other floating panels.
 */
export function bringFloatingToFront(panel) {
  if (!panel) return;
  topFloatingZ = Math.max(40, topFloatingZ + 1);
  panel.style.zIndex = String(topFloatingZ);
}

/**
 * Apply floating fixed geometry to a panel element.
 */
export function applyFloatingGeometry(panel, left, top, width) {
  if (!panel) return;
  panel.classList.add("insp-floating");
  panel.style.left = `${left}px`;
  panel.style.top = `${top}px`;
  panel.style.transform = "";
  if (width) panel.style.width = `${width}px`;
  bringFloatingToFront(panel);
}

/**
 * Apply a GPU-composited translation offset while dragging a floating panel.
 */
export function applyFloatingTransform(panel, dx, dy) {
  if (!panel) return;
  panel.style.transform = `translate3d(${Math.round(dx)}px, ${Math.round(dy)}px, 0)`;
}

/**
 * Commit the final left/top position and clear the drag transform on pointer release.
 */
export function commitFloatingTransform(panel, left, top) {
  if (!panel) return;
  panel.style.left = `${left}px`;
  panel.style.top = `${top}px`;
  panel.style.transform = "";
}

/**
 * Clear floating geometry and dock the panel back into the Inspector sidebar.
 */
export function clearFloatingGeometry(panel) {
  if (!panel) return;
  panel.classList.remove("insp-floating", "insp-dragging");
  panel.style.left = "";
  panel.style.top = "";
  panel.style.transform = "";
  panel.style.width = "";
  panel.style.height = "";
  panel.style.zIndex = "";
}

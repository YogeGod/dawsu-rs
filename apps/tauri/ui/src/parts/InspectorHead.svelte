<script>
  import { onMount } from "svelte";
  import ChevronDown from "../icons/ChevronDown.svg?raw";
  import {
    DRAG_THRESHOLD_PX,
    applyFloatingGeometry,
    applyFloatingTransform,
    applySavedOrder,
    bringFloatingToFront,
    clampFloatingPosition,
    clearFloatingGeometry,
    commitFloatingTransform,
    computeDropIndex,
    findInspectorNodes,
    getSidebarSections,
    saveCurrentOrder,
    shouldDetach,
  } from "../lib/inspectorDock.js";

  // A section's fold header. Folding hides the body but keeps it in the page.
  // Dragging vertically reorders sections in the Inspector; dragging out onto the canvas floats it.
  let { title, body, open = $bindable(true) } = $props();

  let headEl = $state(null);
  let floating = $state(false);
  let suppressNextClick = false;
  let stopDrag = null;

  onMount(() => {
    const nodes = findInspectorNodes(headEl);
    if (nodes?.sidebar) applySavedOrder(nodes.sidebar);
    const panel = nodes?.panel;
    const onPanelPointerDown = () => {
      if (floating && panel) bringFloatingToFront(panel);
    };
    panel?.addEventListener?.("pointerdown", onPanelPointerDown);
    return () => {
      stopDrag?.();
      panel?.removeEventListener?.("pointerdown", onPanelPointerDown);
      if (panel) clearFloatingGeometry(panel);
    };
  });

  function dockBack(event) {
    event?.stopPropagation?.();
    const nodes = findInspectorNodes(headEl);
    if (nodes?.panel) clearFloatingGeometry(nodes.panel);
    floating = false;
  }

  function onClick() {
    if (suppressNextClick) {
      suppressNextClick = false;
      return;
    }
    open = !open;
  }

  function onPointerDown(event) {
    if (event.button !== 0) return;
    const nodes = findInspectorNodes(headEl);
    if (!nodes?.panel) return;
    const { panel, sidebar, host } = nodes;
    const win = globalThis.window;
    if (!win) return;

    const startX = event.clientX;
    const startY = event.clientY;
    const panelRect = panel.getBoundingClientRect();
    const grabX = Math.max(12, Math.min(panelRect.width - 12, startX - panelRect.left));
    const grabY = Math.max(8, Math.min(28, startY - panelRect.top));
    const sidebarRect = sidebar?.getBoundingClientRect?.() ?? null;
    const viewportWidth = win.innerWidth;
    const viewportHeight = win.innerHeight;

    let dragging = false;
    let isFloatingNow = floating;
    const parsedLeft = Number.parseFloat(panel.style.left);
    const parsedTop = Number.parseFloat(panel.style.top);
    let baseLeft = Number.isFinite(parsedLeft) ? parsedLeft : Math.round(panelRect.left);
    let baseTop = Number.isFinite(parsedTop) ? parsedTop : Math.round(panelRect.top);
    let floatWidth = panelRect.width;
    let floatHeight = panelRect.height;
    let pendingLeft = baseLeft;
    let pendingTop = baseTop;
    let hasPendingMove = false;
    let rafId = 0;

    if (isFloatingNow) bringFloatingToFront(panel);

    const flushFloatingTransform = () => {
      rafId = 0;
      if (!hasPendingMove) return;
      applyFloatingTransform(panel, pendingLeft - baseLeft, pendingTop - baseTop);
    };

    const onMove = moveEvent => {
      const dx = moveEvent.clientX - startX;
      const dy = moveEvent.clientY - startY;
      if (!dragging) {
        if (Math.hypot(dx, dy) < DRAG_THRESHOLD_PX) return;
        dragging = true;
        suppressNextClick = true;
        panel.classList.add("insp-dragging");
        host?.classList?.add?.("insp-host-dragging");
      }

      if (!isFloatingNow && sidebarRect && shouldDetach(moveEvent.clientX, sidebarRect)) {
        isFloatingNow = true;
        floating = true;
        const width = Math.max(240, Math.round(panelRect.width));
        const pos = clampFloatingPosition(
          moveEvent.clientX - grabX,
          moveEvent.clientY - grabY,
          width,
          panelRect.height,
          viewportWidth,
          viewportHeight,
        );
        applyFloatingGeometry(panel, pos.left, pos.top, width);
        baseLeft = pos.left;
        baseTop = pos.top;
        floatWidth = width;
        floatHeight = panelRect.height;
        pendingLeft = pos.left;
        pendingTop = pos.top;
        hasPendingMove = false;
        return;
      }

      if (isFloatingNow) {
        const pos = clampFloatingPosition(
          moveEvent.clientX - grabX,
          moveEvent.clientY - grabY,
          floatWidth,
          floatHeight,
          viewportWidth,
          viewportHeight,
        );
        pendingLeft = pos.left;
        pendingTop = pos.top;
        hasPendingMove = true;
        if (!rafId) {
          if (typeof win.requestAnimationFrame === "function") {
            rafId = win.requestAnimationFrame(flushFloatingTransform);
          } else {
            flushFloatingTransform();
          }
        }
        return;
      }

      if (sidebar && host) {
        const sections = getSidebarSections(sidebar);
        const visible = sections.filter(
          s => s.el === host || (s.el.getBoundingClientRect().height > 0 && !s.el.querySelector(".insp-floating")),
        );
        const draggedVisibleIndex = visible.findIndex(s => s.el === host);
        if (draggedVisibleIndex < 0 || visible.length <= 1) return;
        // In Keysounds mode the dragged section's host has no box of its own (inspector.css gives it
        // display: contents for the pinned footer): its panel stands in, or every move would reorder.
        const rects = visible.map(s => (s.el === host ? panel : s.el).getBoundingClientRect());
        const targetVisibleIndex = computeDropIndex(rects, moveEvent.clientY, draggedVisibleIndex);
        if (targetVisibleIndex !== draggedVisibleIndex) {
          const targetEl = visible[targetVisibleIndex].el;
          if (targetVisibleIndex < draggedVisibleIndex) {
            sidebar.insertBefore(host, targetEl);
          } else {
            sidebar.insertBefore(host, targetEl.nextSibling);
          }
        }
      }
    };

    const cleanup = () => {
      if (rafId) {
        win.cancelAnimationFrame?.(rafId);
        rafId = 0;
      }
      if (isFloatingNow && hasPendingMove) {
        commitFloatingTransform(panel, pendingLeft, pendingTop);
        hasPendingMove = false;
      } else if (isFloatingNow) {
        panel.style.transform = "";
      }
      win.removeEventListener("pointermove", onMove);
      win.removeEventListener("pointerup", onUp);
      win.removeEventListener("pointercancel", onUp);
      panel.classList.remove("insp-dragging");
      host?.classList?.remove?.("insp-host-dragging");
      stopDrag = null;
    };

    const onUp = () => {
      cleanup();
      if (dragging && !isFloatingNow && sidebar) {
        saveCurrentOrder(sidebar);
      }
      if (dragging) {
        // Clear the click guard on the next tick in case pointerup ended outside the button.
        win.setTimeout(() => { suppressNextClick = false; }, 0);
      }
    };

    stopDrag?.();
    stopDrag = cleanup;
    win.addEventListener("pointermove", onMove, { passive: true });
    win.addEventListener("pointerup", onUp);
    win.addEventListener("pointercancel", onUp);
  }
</script>

<h2 class="insp-title" class:insp-title-floating={floating}>
  <button type="button" class="insp-head" aria-expanded={open} aria-controls={body} bind:this={headEl}
    onpointerdown={onPointerDown} onclick={onClick}>
    <span class="insp-grip" aria-hidden="true"><svg viewBox="0 0 8 14" focusable="false"><circle cx="2" cy="3" r="1"/><circle cx="6" cy="3" r="1"/><circle cx="2" cy="7" r="1"/><circle cx="6" cy="7" r="1"/><circle cx="2" cy="11" r="1"/><circle cx="6" cy="11" r="1"/></svg></span><span class="insp-chevron" class:insp-folded={!open}>{@html ChevronDown}</span>{title}
  </button>
  {#if floating}
    <button type="button" class="insp-dock-btn" aria-label="Dock {title} back into Inspector"
      title="Dock back into Inspector" onclick={dockBack}>↩</button>
  {/if}
</h2>

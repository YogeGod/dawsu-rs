"use strict";

(() => {
  const labels = { browser: "Browser", timeline: "Timeline", inspector: "Inspector" };
  // A group holds one panel; it keeps the `tabs` list shape so the helpers stay small.
  const group = (tabs, active = tabs[0]) => ({ tabs, active });
  const split = (axis, first, second, basis = "50%") => ({ axis, first, second, size: { basis } });
  // A folded side panel is a 44 px rail (v3 --rail-width).
  const railWidth = 44;

  // v3: Browser | Timeline | Inspector.
  function defaultLayout() {
    return split("columns", group(["browser"]),
      split("columns", group(["timeline"]), group(["inspector"]), "calc(100% - 6px - var(--dock-inspector-width))"),
      "var(--dock-browser-width)");
  }

  function withChildren(node, first, second) {
    if (!first || !second) return first ?? second;
    return { ...node, first, second };
  }

  function mapGroups(node, visit) {
    if (node.tabs) return visit(node);
    return withChildren(node, mapGroups(node.first, visit), mapGroups(node.second, visit));
  }

  function visibleLayout(layout, visible) {
    return mapGroups(layout, node => {
      const tabs = node.tabs.filter(panel => visible.has(panel));
      return tabs.length ? group(tabs, tabs.includes(node.active) ? node.active : tabs[0]) : null;
    });
  }

  function movePanel(layout, panel, target, placement) {
    if (panel === target) return layout;
    const removed = mapGroups(layout, node => node.tabs.includes(panel) ? null : node);
    return mapGroups(removed, node => {
      if (!node.tabs.includes(target)) return node;
      const incoming = group([panel]);
      if (placement === "left") return split("columns", incoming, node);
      if (placement === "above") return split("rows", incoming, node);
      return split(placement === "below" ? "rows" : "columns", node, incoming);
    });
  }

  function minimum(node, axis, folded = new Set()) {
    if (node.tabs) {
      if (node.tabs.every(panel => folded.has(panel))) return railWidth;
      const sizes = axis === "columns" ? { browser: 170, timeline: 274, inspector: 208 }
        : { browser: 86, timeline: 222, inspector: 86 };
      return Math.max(...node.tabs.map(panel => sizes[panel]));
    }
    const first = minimum(node.first, axis, folded);
    const second = minimum(node.second, axis, folded);
    return node.axis === axis ? first + second + 6 : Math.max(first, second);
  }

  // What the frame shows: the Browser "open", "folded" (rail), "classic" (inside the Inspector) or
  // "off", and the Inspector "open" or "folded". Below 1180 px the tree Browser folds for the session
  // unless the mapper opened it there (override true). The classic Browser needs an open Inspector;
  // with the Inspector folded it falls back to the tree panel, and it never draws a left rail.
  function effectivePanels({ browserOpen, inspectorOpen, classic }, narrow = false, override = null) {
    const inspector = inspectorOpen ? "open" : "folded";
    const treeOpen = narrow ? override === true : browserOpen;
    if (classic && inspectorOpen) return { browser: browserOpen ? "classic" : "off", inspector };
    if (classic) return { browser: browserOpen && treeOpen ? "open" : "off", inspector };
    return { browser: treeOpen ? "open" : "folded", inspector };
  }

  // What folding or opening a panel does: a saved `panel` action, or, for the tree Browser while
  // the window is narrow, only the session override (never sent, so never saved).
  function togglePlan(panel, prefs, narrow = false, override = null) {
    const on = !(panel === "browser" ? prefs.browserOpen : panel === "inspector" ? prefs.inspectorOpen : prefs.controllerOpen);
    const save = { act: { kind: "panel", panel, on } };
    if (panel !== "browser" || !narrow) return save;
    const shown = effectivePanels(prefs, narrow, override).browser;
    if (shown === "classic" || (prefs.classic && !prefs.browserOpen)) return save;
    return { override: shown === "open" ? null : true };
  }

  function make(tag, className, attrs = {}) {
    const element = document.createElement(tag);
    element.className = className;
    for (const [name, value] of Object.entries(attrs)) element.setAttribute(name, value);
    return element;
  }

  function attach(root) {
    const byId = id => document.getElementById(id);
    const panels = {
      browser: byId("tree-browser-host"),
      timeline: byId("editor-arrange"),
      inspector: byId("editor-inspector"),
    };
    const classicHost = byId("classic-browser-host");
    const sampleBrowser = byId("sample-browser");
    // History's list is a Svelte part mounted after this script starts, so the areas are looked up each time.
    const scrollAreas = () => [...Object.values(panels), byId("scroll"), byId("history"), byId("sample-browser-entries")].filter(Boolean);
    const scroll = new Map();
    const hiddenBars = new Set();
    let layout = defaultLayout();
    let prefs = { browserStyle: "tree", browserOpen: false, inspectorOpen: true };
    // Narrow windows fold the tree Browser for the session; opening it there is an override that
    // clears when the width crosses back. Neither is sent to the editor, so neither is saved.
    const narrowQuery = window.matchMedia?.("(max-width: 1179px)");
    let narrow = !!narrowQuery?.matches;
    let override = null;
    let shownKey = null;
    // What the frame last drew; render() diffs against it to move focus off a panel that folds or opens.
    let drawn = null;
    const foldButtons = {
      browser: { fold: byId("browser-fold"), open: byId("browser-unfold") },
      inspector: { fold: byId("inspector-fold"), open: byId("inspector-unfold") },
    };
    const switches = { browser: byId("view-browser"), inspector: byId("view-inspector") };
    let renders = 0;
    const effective = () => effectivePanels({ browserOpen: prefs.browserOpen, inspectorOpen: prefs.inspectorOpen,
      classic: prefs.browserStyle === "classic" }, narrow, override);

    let activeResize = null;
    function trackResize(grip, prepare) {
      grip.addEventListener("pointerdown", event => {
        if (event.button !== 0 || !event.isPrimary) return;
        event.preventDefault();
        grip.focus({ preventScroll: true });
        activeResize = { id: event.pointerId, update: prepare(event) };
      });
    }
    document.addEventListener("pointermove", event => {
      if (activeResize?.id !== event.pointerId) return;
      if (event.buttons & 1) activeResize.update(event);
      else activeResize = null;
    }, true);
    for (const type of ["pointerup", "pointercancel"]) {
      document.addEventListener(type, event => {
        if (activeResize?.id === event.pointerId) activeResize = null;
      }, true);
    }
    window.addEventListener("blur", () => { activeResize = null; });

    function move(panel, target, placement) {
      layout = movePanel(layout, panel, target, placement);
      render();
    }

    const foldedGroup = (node, folded) => !!node.tabs && node.tabs.every(panel => folded.has(panel));

    function renderSplit(node, folded) {
      const row = node.axis === "rows";
      const box = make("div", "dock-split");
      const firstFolded = foldedGroup(node.first, folded);
      const secondFolded = foldedGroup(node.second, folded);
      const locked = firstFolded || secondFolded;
      const grip = make("div", "dock-grip", locked ? { "aria-hidden": "true" } : {
        role: "separator", tabindex: "0", "data-dock-control": "",
        "aria-orientation": row ? "horizontal" : "vertical",
        "aria-label": row ? "Resize panels above and below" : "Resize panels to the left and right",
      });
      box.dataset.axis = node.axis;
      box.toggleAttribute("data-locked", locked);
      const first = renderNode(node.first, folded);
      box.append(first, grip, renderNode(node.second, folded));
      // A folded side is its rail. The grip rests until the panel opens again, and the mapper's
      // width (node.size.basis) is kept for then.
      if (locked) {
        box.style.setProperty("--dock-first", firstFolded ? "var(--rail-width)" : "calc(100% - var(--rail-width))");
        return box;
      }
      const firstMin = minimum(node.first, node.axis, folded);
      const secondMin = minimum(node.second, node.axis, folded);
      const dimension = row ? "height" : "width";
      const coordinate = row ? "clientY" : "clientX";

      function updateSize() {
        const basis = node.size.basis;
        box.style.setProperty("--dock-first", "clamp(min(" + firstMin + "px, 48%), " + basis +
          ", calc(100% - 6px - min(" + secondMin + "px, 48%)))");
      }
      function describeSize() {
        const size = box.getBoundingClientRect()[dimension] - 6;
        if (size <= 0) return;
        grip.setAttribute("aria-valuemin", String(Math.round(Math.min(firstMin, (size + 6) * .48) / size * 100)));
        grip.setAttribute("aria-valuemax", String(Math.round((size - Math.min(secondMin, (size + 6) * .48)) / size * 100)));
        grip.setAttribute("aria-valuenow", String(Math.round(first.getBoundingClientRect()[dimension] / size * 100)));
      }
      function resize(pixels) {
        const size = box.getBoundingClientRect()[dimension] - 6;
        if (size <= 0) return;
        const percent = Math.max(0, Math.min(pixels / (size + 6), 1)) * 100;
        node.size.basis = percent + "%";
        updateSize();
        describeSize();
      }
      updateSize();
      grip.addEventListener("focus", describeSize);
      trackResize(grip, event => {
        const edge = row ? "top" : "left";
        const offset = event[coordinate] - box.getBoundingClientRect()[edge] - first.getBoundingClientRect()[dimension];
        return next => resize(next[coordinate] - box.getBoundingClientRect()[edge] - offset);
      });
      grip.addEventListener("keydown", event => {
        if (event.altKey || event.ctrlKey || event.metaKey) return;
        const earlier = row ? "ArrowUp" : "ArrowLeft";
        const later = row ? "ArrowDown" : "ArrowRight";
        if (![earlier, later, "Home", "End"].includes(event.key)) return;
        event.preventDefault();
        const size = first.getBoundingClientRect()[dimension];
        if (event.key === "Home" || event.key === "End") {
          resize(event.key === "Home" ? 0 : box.getBoundingClientRect()[dimension]);
          return;
        }
        resize(size + (event.key === earlier ? -1 : 1) * (event.shiftKey ? 32 : 12));
      });
      return box;
    }

    function renderNode(node, folded) {
      if (node.axis) return renderSplit(node, folded);
      const box = make("div", "dock-group");
      const [panel] = node.tabs;
      box.dataset.dockTarget = panel;
      box.toggleAttribute("data-folded", folded.has(panel));
      const content = panels[panel];
      content.hidden = false;
      box.append(content);
      return box;
    }

    // Focus inside a panel that just folded moves to its rail's open button, and to its head's fold
    // button when it opens, whichever path changed it (a switch, a rail, a narrow window).
    function render() {
      const focused = document.activeElement;
      const state = effective();
      const changed = drawn ? ["browser", "inspector"].filter(panel => drawn[panel] !== state[panel]) : [];
      drawn = state;
      const classic = state.browser === "classic";
      const folded = new Set();
      if (state.browser === "folded") folded.add("browser");
      if (state.inspector === "folded") folded.add("inspector");
      const shown = new Set(["timeline", "inspector"]);
      if (state.browser === "open" || state.browser === "folded") shown.add("browser");
      for (const panel of ["browser", "inspector"]) {
        if (hiddenBars.has(panels[panel])) shown.delete(panel);
      }
      // The sample list can sit in the Inspector (classic), so it counts as the Browser first.
      const focusedPanel = sampleBrowser.contains(focused) ? "browser"
        : Object.keys(panels).find(panel => panels[panel].contains(focused));
      for (const area of scrollAreas()) {
        if (area.getClientRects().length) scroll.set(area, [area.scrollLeft, area.scrollTop]);
      }
      root.dataset.docking = "";
      const host = classic ? classicHost : panels.browser;
      if (sampleBrowser.parentNode !== host) host.append(sampleBrowser);
      // A folded panel shows only its rail; what it holds stays in the page, hidden.
      for (const panel of ["browser", "inspector"]) {
        panels[panel].toggleAttribute("data-folded", folded.has(panel));
        for (const child of panels[panel].children) {
          if (child === classicHost) continue;
          child.hidden = child.matches(".panel-rail") ? !folded.has(panel) : folded.has(panel);
        }
      }
      classicHost.hidden = !classic;
      const hidden = make("div", "");
      hidden.hidden = true;
      for (const [panel, content] of Object.entries(panels)) {
        if (!shown.has(panel)) {
          content.hidden = true;
          hidden.append(content);
        }
      }
      root.replaceChildren(renderNode(visibleLayout(layout, shown), folded), hidden);
      for (const [area, [left, top]] of scroll) {
        if (area.getClientRects().length) area.scrollTo(left, top);
      }
      renders++;
      const moved = changed.find(panel => panel === focusedPanel);
      if (moved) foldButtons[moved][folded.has(moved) ? "open" : "fold"]?.focus({ preventScroll: true });
      else if (focusedPanel && focused.getClientRects().length) focused.focus({ preventScroll: true });
      else if (focusedPanel) (switches[focusedPanel] ?? byId("grid"))?.focus({ preventScroll: true });
    }

    // Where a dragged panel lands: the edge of the target panel nearest the pointer.
    function dropAt(event, panel) {
      const box = event.target.closest("[data-dock-target]");
      if (!box) return null;
      const target = box.dataset.dockTarget;
      if (target === panel || box.hasAttribute("data-folded")) return null;
      const bounds = box.getBoundingClientRect();
      const x = (event.clientX - bounds.left) / bounds.width;
      const y = (event.clientY - bounds.top) / bounds.height;
      const placement = Math.min(y, 1 - y) < Math.min(x, 1 - x) ? (y < .5 ? "above" : "below") : (x < .5 ? "left" : "right");
      return { target, placement, bounds };
    }

    function dockPlacement({ placement, bounds }) {
      const halfWidth = placement === "left" || placement === "right";
      const halfHeight = placement === "above" || placement === "below";
      return {
        x: bounds.left + (placement === "right" ? bounds.width / 2 : 0),
        y: bounds.top + (placement === "below" ? bounds.height / 2 : 0),
        width: bounds.width / (halfWidth ? 2 : 1),
        height: bounds.height / (halfHeight ? 2 : 1),
      };
    }

    const workspace = byId("workspace");
    const editor = byId("layout-editor");
    const ribbon = byId("editor-ribbon");
    const ribbonPicker = byId("layout-ribbon-group");
    const ribbonGroups = () => [...ribbon.children].filter(item => item.matches(".ribbon-group"));
    const initialRibbon = ribbonGroups();
    // Inspector sections move whole, like ribbon groups: Svelte owns what is inside a part's section.
    const inspectorSections = () => [...(panels.inspector?.children ?? [])].filter(item => item.matches("section"));
    const initialInspector = [...(panels.inspector?.children ?? [])];
    let draft = null;

    const header = byId("editor-header");
    const status = byId("statusbar");
    const arrange = panels.timeline;
    const overview = arrange.querySelector(".overview");
    const ruler = arrange.querySelector(".ruler");
    const grid = byId("scroll");
    const controller = byId("volume-controller-panel");
    const laneColumn = arrange.querySelector(".ruler-sidebar");
    const defaultFrame = [header, ribbon, root, status];
    const defaultTimeline = [ruler, grid, controller, overview];
    let frameOrder = defaultFrame.slice();
    let timelineOrder = defaultTimeline.slice();

    const barChoices = new Map([
      [header, "Header"], [ribbon, "Ribbon"], [status, "Status bar"],
      [overview, "Overview"], [ruler, "Ruler and waveform"], [controller, "Volume controller"],
      [laneColumn, "Lane sidebar"], [panels.browser, "Browser"], [panels.inspector, "Inspector"],
    ]);
    const laneRows = new Map([[overview, "minmax(20px,max-content)"], [ruler, "var(--ruler-row)"],
      [grid, "minmax(0,1fr)"], [controller, "auto"]]);
    const barPicker = byId("layout-bar");
    const barHidden = byId("layout-bar-hidden");
    const barEarlier = byId("layout-bar-earlier");
    const barLater = byId("layout-bar-later");
    const recovery = make("div", "layout-recovery");
    recovery.hidden = true;
    workspace.append(recovery);
    // Edit layout… lives in the View menu; this button only brings it back while the header is hidden.
    const layoutEntry = byId("layout-edit-entry");
    recovery.append(layoutEntry);
    for (const [bar, label] of barChoices) {
      const option = make("option", "");
      option.textContent = label;
      option.bar = bar;
      barPicker.append(option);
    }

    function barStack(bar) {
      if (frameOrder.includes(bar)) return frameOrder;
      return timelineOrder.includes(bar) ? timelineOrder : null;
    }

    function refreshBarPicker() {
      const bar = barPicker.selectedOptions[0]?.bar;
      const stack = barStack(bar)?.filter(item => item === bar || !item.hidden && !hiddenBars.has(item)) ?? [];
      const index = stack.indexOf(bar);
      barHidden.checked = hiddenBars.has(bar);
      barEarlier.disabled = index <= 0;
      barLater.disabled = index < 0 || index >= stack.length - 1;
    }

    function syncTimelineRows() {
      const rows = timelineOrder.filter(row => !row.hidden && !hiddenBars.has(row));
      arrange.style.setProperty("--layout-timeline-rows", rows.map(row => laneRows.get(row)).join(" "));
      for (const row of [overview, ruler, controller]) {
        row.toggleAttribute("data-layout-size-from-top", timelineOrder.indexOf(row) > timelineOrder.indexOf(grid));
      }
    }

    function applyBarLayout() {
      if (workspace.hidden) {
        byId("home").before(header);
        workspace.after(status);
      } else {
        workspace.append(...frameOrder);
      }
      arrange.append(...timelineOrder);
      for (const bar of barChoices.keys()) {
        if (barStack(bar)) bar.toggleAttribute("data-layout-bar-hidden", hiddenBars.has(bar) && !workspace.hidden);
        if (frameOrder.includes(bar)) {
          bar.toggleAttribute("data-layout-size-from-top", frameOrder.indexOf(bar) > frameOrder.indexOf(root));
        }
      }
      arrange.toggleAttribute("data-layout-lane-sidebar-hidden", hiddenBars.has(laneColumn));
      recovery.hidden = workspace.hidden || !hiddenBars.has(header);
      layoutEntry.hidden = recovery.hidden;
      syncTimelineRows();
      refreshBarPicker();
    }

    function placeBar(bar, target, after) {
      const stack = barStack(bar);
      stack.splice(stack.indexOf(bar), 1);
      stack.splice(stack.indexOf(target) + Number(after), 0, bar);
      applyBarLayout();
    }

    function moveSelectedBar(direction) {
      const bar = barPicker.selectedOptions[0].bar;
      const stack = barStack(bar)?.filter(item => item === bar || !item.hidden && !hiddenBars.has(item)) ?? [];
      const target = stack[stack.indexOf(bar) + direction];
      if (target && target !== bar) placeBar(bar, target, direction > 0);
    }

    barPicker.addEventListener("change", refreshBarPicker);
    barEarlier.addEventListener("click", () => moveSelectedBar(-1));
    barLater.addEventListener("click", () => moveSelectedBar(1));
    barHidden.addEventListener("change", () => {
      const bar = barPicker.selectedOptions[0].bar;
      if (barHidden.checked) hiddenBars.add(bar);
      else hiddenBars.delete(bar);
      applyBarLayout();
      if (bar === panels.browser || bar === panels.inspector) render();
    });
    new MutationObserver(syncTimelineRows).observe(controller, { attributeFilter: ["hidden"] });
    new MutationObserver(() => {
      if (workspace.hidden && draft) finishEditing(false);
      else applyBarLayout();
    }).observe(workspace, { attributeFilter: ["hidden"] });

    function refreshRibbonPicker() {
      const previous = ribbonPicker.value;
      const options = ribbonGroups().map(item => {
        const option = document.createElement("option");
        option.value = String(initialRibbon.indexOf(item));
        option.textContent = item.getAttribute("aria-label");
        return option;
      });
      ribbonPicker.replaceChildren(...options);
      if (options.some(option => option.value === previous)) ribbonPicker.value = previous;
      const index = ribbonPicker.selectedIndex;
      byId("layout-left").disabled = index <= 0;
      byId("layout-right").disabled = index < 0 || index >= options.length - 1;
    }

    function moveRibbonGroup(direction) {
      const groups = ribbonGroups();
      const active = initialRibbon[Number(ribbonPicker.value)];
      const destination = groups[groups.indexOf(active) + direction];
      if (!destination || !active) return;
      ribbon.insertBefore(active, direction < 0 ? destination : destination.nextSibling);
      refreshRibbonPicker();
      ribbonPicker.focus();
    }

    const buttonPicker = byId("layout-button");
    const location = byId("layout-button-location");
    const earlier = byId("layout-button-before");
    const later = byId("layout-button-after");
    const hide = byId("layout-button-hidden");
    const sizes = ["width", "height"].map(name => [name, byId(`layout-button-${name}`)]);
    const buttons = new Map();
    let layoutDrag = null;
    const slots = new Map();
    const selectedButton = () => buttonPicker.selectedOptions[0]?.layoutButton;
    const anchorFor = button => {
      const anchor = document.createComment("Button position");
      button.before(anchor);
      return anchor;
    };

    function collectButtons() {
      const regions = [
        ["header", "Header", byId("editor-header")],
        ["ribbon", "Ribbon", ribbon],
        ["ruler", "Ruler", byId("editor-arrange").querySelector(".ruler-controls"), ".ruler-sidebar button"],
        ["inspector", "Inspector", byId("editor-inspector")],
        ["browser", "Browser", byId("sample-browser")],
        ["status", "Status bar", byId("statusbar")],
      ];
      for (const [region, label, host, extra] of regions) {
        const candidates = [...host.querySelectorAll("button")];
        if (extra) candidates.push(...byId("editor-arrange").querySelectorAll(extra));
        for (const button of candidates) {
          if (button === layoutEntry || buttons.has(button) ||
              button.closest("#history, #sample-browser-entries, #editor-menus, .panel-head, .panel-rail, [data-ui-mount]") ||
              (region !== "browser" && button.closest("#sample-browser"))) continue;
          const anchor = anchorFor(button);
          const condition = button.closest("fieldset");
          if (condition) {
            const sync = () => button.toggleAttribute("data-layout-unavailable",
              button.hasAttribute("data-layout-region") && condition.hidden);
            new MutationObserver(sync).observe(condition, { attributeFilter: ["hidden"] });
            new MutationObserver(sync).observe(button, { attributeFilter: ["data-layout-region"] });
          }
          if (button.form && button.type === "submit") button.setAttribute("form", button.form.id);
          if (!button.form && !button.hasAttribute("type")) button.type = "button";
          if (region === "ruler" && button.querySelector("svg")) button.dataset.layoutRulerIcon = "";
          buttons.set(button, { anchor, region });
          button.dataset.layoutMovable = "";
          const option = document.createElement("option");
          const name = button.getAttribute("aria-label") || button.title || button.textContent.trim();
          const context = condition?.querySelector("legend")?.textContent ||
            (region === "inspector" && button.closest("section")?.querySelector("h2")?.textContent);
          option.textContent = [label, context, name].filter(Boolean).join(" · ");
          option.layoutButton = button;
          buttonPicker.append(option);
        }
        const slot = document.createElement("div");
        slot.className = "layout-button-slot";
        if (region === "inspector" || region === "browser") host.prepend(slot);
        else host.append(slot);
        host.dataset.layoutDropRegion = region;
        slots.set(region, slot);
      }
    }

    function neighbours(button) {
      const siblings = [...button.parentElement.children].filter(item => buttons.has(item));
      const index = siblings.indexOf(button);
      return [siblings[index - 1], siblings[index + 1]];
    }

    function refreshButtonPicker() {
      const button = selectedButton();
      if (!button) return;
      location.value = button.dataset.layoutRegion ?? "original";
      hide.checked = button.hasAttribute("data-layout-hidden");
      for (const [name, field] of sizes) field.value = button.getAttribute(`data-layout-${name}`) ?? "";
      const [previous, next] = neighbours(button);
      earlier.disabled = !previous;
      later.disabled = !next;
      for (const item of buttons.keys()) item.toggleAttribute("data-layout-selected", item === button);
    }

    function setButtonSize(button, name, value) {
      if (value === null) {
        button.removeAttribute(`data-layout-${name}`);
        button.style.removeProperty(`--layout-${name}`);
      } else {
        button.setAttribute(`data-layout-${name}`, value);
        button.style.setProperty(`--layout-${name}`, `${value}px`);
      }
    }

    function restoreButton(button, anchor, region, hidden, width, height) {
      anchor.after(button);
      if (region === null) button.removeAttribute("data-layout-region");
      else button.dataset.layoutRegion = region;
      button.toggleAttribute("data-layout-hidden", hidden);
      setButtonSize(button, "width", width);
      setButtonSize(button, "height", height);
    }

    buttonPicker.addEventListener("change", refreshButtonPicker);
    location.addEventListener("change", () => {
      const button = selectedButton();
      const slot = slots.get(location.value);
      if (slot) {
        slot.append(button);
        button.dataset.layoutRegion = location.value;
      } else {
        buttons.get(button).anchor.after(button);
        delete button.dataset.layoutRegion;
      }
      refreshButtonPicker();
    });
    earlier.addEventListener("click", () => {
      const button = selectedButton();
      neighbours(button)[0]?.before(button);
      refreshButtonPicker();
    });
    later.addEventListener("click", () => {
      const button = selectedButton();
      neighbours(button)[1]?.after(button);
      refreshButtonPicker();
    });
    hide.addEventListener("change", () => selectedButton().toggleAttribute("data-layout-hidden", hide.checked));
    for (const [name, field] of sizes) {
      field.addEventListener("change", () => {
        const value = field.value === "" ? null : Math.min(+field.max, Math.max(+field.min, field.valueAsNumber));
        setButtonSize(selectedButton(), name, value);
        refreshButtonPicker();
      });
    }

    function pickButton(button) {
      buttonPicker.selectedIndex = [...buttonPicker.options].findIndex(option => option.layoutButton === button);
      refreshButtonPicker();
      buttonPicker.focus();
    }

    document.addEventListener("click", event => {
      if (!draft) return;
      const button = event.target.closest("button");
      if (!buttons.has(button)) return;
      event.preventDefault();
      event.stopImmediatePropagation();
      pickButton(button);
    }, true);
    const placement = make("div", "layout-placement", { "aria-hidden": "true" });
    placement.hidden = true;
    document.body.append(placement);

    function moveHandle(host, label, row = false) {
      const handle = make("span", "layout-move-handle" + (row ? " layout-row-handle" : ""), { title: label, "aria-hidden": "true" });
      handle.textContent = "⠿";
      host.prepend(handle);
      return handle;
    }

    const barHandles = new Map(initialRibbon.map(group =>
      [moveHandle(group, "Drag " + group.getAttribute("aria-label") + " group"), group]));
    for (const [bar, label] of barChoices) {
      if (barStack(bar)) barHandles.set(moveHandle(bar, "Move " + label + " up or down", timelineOrder.includes(bar)), bar);
    }
    // In Layout… a panel moves by the grip on its head; a folded panel stays put.
    for (const panel of ["browser", "inspector"]) {
      const head = panels[panel].querySelector(".panel-head");
      if (head) barHandles.set(moveHandle(head, "Drag " + labels[panel] + " panel"), panels[panel]);
    }
    for (const section of inspectorSections()) {
      barHandles.set(moveHandle(section, "Drag " + (section.getAttribute("aria-label") ||
        section.querySelector("h2")?.textContent || "Inspector") + " section"), section);
    }

    function layoutDropAt(source, x, y) {
      const under = document.elementFromPoint(x, y);
      if (!under) return null;
      const panel = Object.keys(panels).find(name => panels[name] === source);
      if (panel) {
        if (source.hasAttribute("data-folded")) return null;
        const destination = dropAt({ target: under, clientX: x, clientY: y }, panel);
        return destination && { ...dockPlacement(destination), area: true,
          apply: () => move(panel, destination.target, destination.placement) };
      }
      const stack = barStack(source);
      if (stack) {
        const parent = stack === frameOrder ? workspace : arrange;
        if (!parent.contains(under)) return null;
        const candidates = stack.filter(bar => bar !== source && bar.getClientRects().length);
        const target = candidates.find(bar => {
          const bounds = bar.getBoundingClientRect();
          return y < bounds.top + bounds.height / 2;
        }) ?? candidates.at(-1);
        if (!target) return null;
        const bounds = target.getBoundingClientRect();
        const after = y >= bounds.top + bounds.height / 2;
        return { x: bounds.x, y: after ? bounds.bottom : bounds.top, width: bounds.width, height: 2,
          apply: () => {
            placeBar(source, target, after);
            barPicker.selectedIndex = [...barChoices.keys()].indexOf(source);
            refreshBarPicker();
          } };
      }

      if (inspectorSections().includes(source)) {
        const section = under.closest("#editor-inspector > section");
        if (!section || section === source) return null;
        const bounds = section.getBoundingClientRect();
        const after = y >= bounds.top + bounds.height / 2;
        return { x: bounds.x, y: after ? bounds.bottom : bounds.top, width: bounds.width, height: 2,
          apply: () => (after ? section.after(source) : section.before(source)) };
      }

      const isButton = buttons.has(source);
      const target = under.closest(isButton ? "button" : "#editor-ribbon > .ribbon-group");
      if (target === source) return null;
      if (target && (!isButton || buttons.has(target))) {
        const bounds = target.getBoundingClientRect();
        const horizontal = !isButton || getComputedStyle(target.parentElement).display.includes("flex");
        const after = horizontal ? x >= bounds.left + bounds.width / 2 : y >= bounds.top + bounds.height / 2;
        return {
          x: horizontal && after ? bounds.right : bounds.left,
          y: !horizontal && after ? bounds.bottom : bounds.top,
          width: horizontal ? 2 : bounds.width, height: horizontal ? bounds.height : 2,
          apply: () => {
            if (after) target.after(source);
            else target.before(source);
            if (!isButton) {
              ribbonPicker.value = String(initialRibbon.indexOf(source));
              refreshRibbonPicker();
            } else if (source.parentElement === buttons.get(source).anchor.parentElement) {
              delete source.dataset.layoutRegion;
            } else {
              source.dataset.layoutRegion = target.dataset.layoutRegion ?? buttons.get(target).region;
            }
          },
        };
      }
      if (!isButton) return null;
      const host = under.closest("[data-layout-drop-region]");
      if (!host) return null;
      const region = host.dataset.layoutDropRegion;
      const slot = slots.get(region);
      const bounds = host.getBoundingClientRect();
      return { x: bounds.x + 2, y: bounds.y + 2, width: bounds.width - 4, height: bounds.height - 4,
        area: true, apply: () => { slot.append(source); source.dataset.layoutRegion = region; } };
    }

    function clearLayoutDrag() {
      layoutDrag = null;
      placement.hidden = true;
      document.body.removeAttribute("data-layout-dragging");
    }

    document.addEventListener("pointerdown", event => {
      if (!draft || layoutDrag || event.button !== 0 || !event.isPrimary) return;
      const handle = event.target.closest(".layout-move-handle, button");
      const source = barHandles.get(handle) ??
        (buttons.has(handle) ? handle : null);
      if (!source) return;
      layoutDrag = { id: event.pointerId, source, x: event.clientX, y: event.clientY, moved: false };
      event.preventDefault();
    }, true);
    document.addEventListener("dragstart", event => {
      if (layoutDrag) event.preventDefault();
    }, true);
    document.addEventListener("pointermove", event => {
      if (layoutDrag?.id !== event.pointerId) return;
      if (!(event.buttons & 1)) {
        clearLayoutDrag();
        return;
      }
      const { source, x, y, moved } = layoutDrag;
      if (!moved && Math.hypot(event.clientX - x, event.clientY - y) < 5) return;
      layoutDrag.moved = true;
      document.body.setAttribute("data-layout-dragging", "");
      const drop = layoutDropAt(source, event.clientX, event.clientY);
      placement.hidden = !drop;
      if (drop) {
        placement.toggleAttribute("data-area", Boolean(drop.area));
        placement.style.cssText = `left:${drop.x}px;top:${drop.y}px;width:${drop.width}px;height:${drop.height}px`;
      }
    }, true);
    document.addEventListener("pointerup", event => {
      if (layoutDrag?.id !== event.pointerId) return;
      const { source, moved } = layoutDrag;
      const drop = moved && layoutDropAt(source, event.clientX, event.clientY);
      clearLayoutDrag();
      if (!drop) return;
      drop.apply();
      if (buttons.has(source)) pickButton(source);
    }, true);
    document.addEventListener("pointercancel", event => {
      if (layoutDrag?.id === event.pointerId) clearLayoutDrag();
    }, true);
    window.addEventListener("blur", clearLayoutDrag);

    const layoutSizes = [];
    function resizeGrip(host, label, variable, { measure = host, width = false, direction, max }) {
      const column = width ? " layout-size-column" : "";
      const grip = make("div", "layout-size-grip" + column, {
        role: "separator", tabindex: "0", "data-dock-control": "",
        "aria-orientation": width ? "vertical" : "horizontal", "aria-label": "Resize " + label,
        title: "Resize " + label + ". Double-click to reset.",
      });
      const property = width ? "width" : "height";
      const coordinate = width ? "clientX" : "clientY";
      const current = () => measure.getBoundingClientRect()[property];
      const limit = () => Math.max(0, Math.round(max()));
      const set = value => {
        if (value === "") host.style.removeProperty(variable);
        else host.style.setProperty(variable, Math.max(0, Math.min(Math.round(value), limit())) + "px");
        grip.setAttribute("aria-valuemax", String(limit()));
        grip.setAttribute("aria-valuenow", String(Math.round(current())));
      };
      grip.setAttribute("aria-valuemin", "0");
      host.classList.add("layout-size-host");
      host.append(grip);
      layoutSizes.push({ host, variable, set });
      grip.addEventListener("focus", () => {
        grip.setAttribute("aria-valuemax", String(limit()));
        grip.setAttribute("aria-valuenow", String(Math.round(current())));
      });
      trackResize(grip, event => {
        const size = current();
        const origin = event[coordinate];
        const sign = direction?.() ?? 1;
        return next => set(size + (next[coordinate] - origin) * sign);
      });
      grip.addEventListener("dblclick", () => set(""));
      grip.addEventListener("keydown", event => {
        const earlier = width ? "ArrowLeft" : "ArrowUp";
        const later = width ? "ArrowRight" : "ArrowDown";
        if (event.ctrlKey || event.altKey || event.metaKey || ![earlier, later, "Home", "End"].includes(event.key)) return;
        event.preventDefault();
        if (event.key === "Home") set("");
        else if (event.key === "End") set(limit());
        else set(current() + (event.key === earlier ? -1 : 1) * (direction?.() ?? 1) * (event.shiftKey ? 24 : 8));
      });
    }

    const chromeMax = () => window.innerHeight * .18;
    const resizeDirection = bar => () => bar.hasAttribute("data-layout-size-from-top") ? -1 : 1;
    for (const bar of [header, ribbon, status, overview]) {
      resizeGrip(bar, barChoices.get(bar), "--layout-bar-height", {
        max: chromeMax, direction: resizeDirection(bar),
      });
    }
    resizeGrip(controller, "Volume controller", "--layout-controller-height", {
      direction: resizeDirection(controller), max: () => arrange.clientHeight * .5,
    });
    resizeGrip(arrange, "Lane sidebar", "--layout-lane-width", {
      measure: byId("lanes"), width: true, max: () => Math.max(99, arrange.clientWidth - 140),
    });
    resizeGrip(classicHost, "Classic Browser list", "--layout-classic-list-height", {
      measure: byId("sample-browser-entries"), max: () => window.innerHeight * .6,
    });

    function snapshotLayoutSizes() {
      return layoutSizes.map(({ host, variable }) => parseFloat(host.style.getPropertyValue(variable)) || "");
    }

    function restoreLayoutSizes(sizes = []) {
      layoutSizes.forEach(({ set }, index) => set(sizes[index] ?? ""));
    }

    layoutEntry.addEventListener("click", () => draft ? finishEditing(false) : editLayout());

    function editLayout() {
      if (workspace.hidden) return;
      if (!draft) {
        if (!buttons.size) collectButtons();
        draft = {
          buttons: [...buttons.keys()].map(button => [button, anchorFor(button),
            button.getAttribute("data-layout-region"), button.hasAttribute("data-layout-hidden"),
            button.getAttribute("data-layout-width"), button.getAttribute("data-layout-height")]),
          layout: structuredClone(layout),
          sizes: snapshotLayoutSizes(),
          waveform: waveformHeight,
          visibleWaveform: visibleWaveformHeight,
          ribbon: ribbonGroups(),
          inspector: [...panels.inspector.children],
          frameOrder: [...frameOrder],
          timelineOrder: [...timelineOrder],
          hiddenBars: [...hiddenBars],
        };
        document.body.setAttribute("data-layout-editing", "");
        editor.hidden = false;
        refreshBarPicker();
        refreshRibbonPicker();
        refreshButtonPicker();
      }
      buttonPicker.focus();
    }

    function finishEditing(keep) {
      if (!draft) return;
      if (!keep) {
        layout = draft.layout;
        ribbon.append(...draft.ribbon);
        panels.inspector.append(...draft.inspector);
        frameOrder = draft.frameOrder;
        timelineOrder = draft.timelineOrder;
        hiddenBars.clear();
        for (const bar of draft.hiddenBars) hiddenBars.add(bar);
        applyBarLayout();
        restoreLayoutSizes(draft.sizes);
        setWaveformHeight(draft.waveform);
        visibleWaveformHeight = draft.visibleWaveform;
        render();
      }
      for (const record of draft.buttons) {
        if (!keep) restoreButton(...record);
        record[1].remove();
        record[0].removeAttribute("data-layout-selected");
      }
      draft = null;
      editor.hidden = true;
      clearLayoutDrag();
      activeResize = null;
      document.body.removeAttribute("data-layout-editing");
      (recovery.hidden ? byId("menu-view") : layoutEntry).focus();
    }

    ribbonPicker.addEventListener("change", refreshRibbonPicker);
    byId("layout-left").addEventListener("click", () => moveRibbonGroup(-1));
    byId("layout-right").addEventListener("click", () => moveRibbonGroup(1));
    byId("layout-cancel").addEventListener("click", () => finishEditing(false));
    byId("layout-keep").addEventListener("click", () => finishEditing(true));
    byId("layout-reset").addEventListener("click", () => {
      layout = defaultLayout();
      ribbon.append(...initialRibbon);
      panels.inspector.append(...initialInspector);
      frameOrder = defaultFrame.slice();
      timelineOrder = defaultTimeline.slice();
      hiddenBars.clear();
      applyBarLayout();
      restoreLayoutSizes();
      setWaveformHeight(defaultWaveformHeight);
      for (const [button, { anchor }] of buttons) restoreButton(button, anchor, null, false, null, null);
      refreshButtonPicker();
      refreshRibbonPicker();
      render();
    });
    editor.addEventListener("keydown", event => {
      if (event.key === "Escape" && !event.defaultPrevented && event.target.tagName !== "SELECT") {
        event.preventDefault();
        finishEditing(false);
      }
    });

    const stateKey = () => JSON.stringify([effective(), narrow, override]);
    // app.js hands over the saved preferences on every show(); the frame redraws only when what it
    // shows changes (fold per panel, classic, narrow, override).
    const update = next => {
      prefs = { browserStyle: next.browserStyle, browserOpen: !!next.browserOpen, inspectorOpen: !!next.inspectorOpen };
      const key = stateKey();
      if (root.hasAttribute("data-docking") && key === shownKey) return;
      shownKey = key;
      render();
    };
    update.state = () => ({ narrow, override });
    update.setOverride = value => { override = value; };
    update.renders = () => renders;
    narrowQuery?.addEventListener("change", () => {
      narrow = narrowQuery.matches;
      override = null;
      update.onChange?.();
    });
    update.editLayout = editLayout;
    return update;
  }

  window.editorDocking = { defaultLayout, movePanel, visibleLayout, minimum, effectivePanels, togglePlan, attach };
})();

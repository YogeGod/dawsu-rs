"use strict";

(() => {
  const labels = { browser: "Browser", timeline: "Timeline", inspector: "Inspector" };
  const group = (tabs, active = tabs[0]) => ({ tabs, active });
  const split = (axis, first, second, basis = "50%") => ({ axis, first, second, size: { basis } });

  function defaultLayout() {
    return split("columns", group(["browser"]),
      split("columns", group(["inspector"]), group(["timeline"]), "var(--dock-inspector-width)"),
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
    const removed = mapGroups(layout, node => {
      if (!node.tabs.includes(panel)) return node;
      const tabs = node.tabs.filter(value => value !== panel);
      return tabs.length ? group(tabs, node.active === panel ? tabs[0] : node.active) : null;
    });
    return mapGroups(removed, node => {
      if (!node.tabs.includes(target)) return node;
      if (placement === "tabs") return group([...node.tabs, panel], panel);
      const incoming = group([panel]);
      if (placement === "left") return split("columns", incoming, node);
      if (placement === "above") return split("rows", incoming, node);
      return split(placement === "below" ? "rows" : "columns", node, incoming);
    });
  }

  function minimum(node, axis) {
    if (node.tabs) {
      const sizes = axis === "columns" ? { browser: 170, timeline: 274, inspector: 208 }
        : { browser: 86, timeline: 222, inspector: 86 };
      return Math.max(...node.tabs.map(panel => sizes[panel]));
    }
    const first = minimum(node.first, axis);
    const second = minimum(node.second, axis);
    return node.axis === axis ? first + second + 6 : Math.max(first, second);
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
    const scrollAreas = [...Object.values(panels), byId("scroll"), byId("history"), byId("sample-browser-entries")];
    const scroll = new Map();
    const visible = new Set(["timeline"]);
    const tabButtons = new Map();
    const preview = make("div", "dock-preview", { "aria-hidden": "true" });
    preview.hidden = true;
    const hiddenBars = new Set();
    let layout = defaultLayout();
    let browser = "off";
    let detached = false;
    let dragged = null;

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

    function activate(panel) {
      layout = mapGroups(layout, node => node.tabs.includes(panel) ? group(node.tabs, panel) : node);
    }

    function move(panel, target, placement) {
      layout = movePanel(layout, panel, target, placement);
      detached = true;
      render(panel);
    }

    function strip(node, classic = false) {
      const bar = make("div", "dock-bar", { "data-dock-control": "" });
      const tabs = make("div", "dock-tabs", { role: "tablist", "aria-label": "Docked panels" });
      bar.append(tabs);
      for (const [index, panel] of node.tabs.entries()) {
        const tab = make("button", "dock-tab", {
          type: "button", role: "tab", draggable: "true", id: "dock-tab-" + panel,
          "aria-selected": String(panel === node.active),
          "aria-controls": classic ? sampleBrowser.id : panels[panel].id,
        });
        tab.textContent = labels[panel];
        tab.tabIndex = panel === node.active ? 0 : -1;
        tabButtons.set(panel, tab);
        tab.addEventListener("click", () => {
          if (panel !== node.active) {
            activate(panel);
            render(panel);
          }
        });
        tab.addEventListener("keydown", event => {
          const { key } = event;
          if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return;
          if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(key)) return;
          event.preventDefault();
          const step = key === "ArrowLeft" ? -1 : 1;
          const next = key === "Home" ? 0 : key === "End" ? node.tabs.length - 1
            : (index + step + node.tabs.length) % node.tabs.length;
          if (next !== index) {
            activate(node.tabs[next]);
            render(node.tabs[next]);
          }
        });
        tab.addEventListener("dragstart", event => {
          dragged = panel;
          event.dataTransfer.effectAllowed = "move";
          event.dataTransfer.setData("application/x-dawsu-dock-panel", panel);
        });
        tabs.append(tab);
      }

      const trigger = make("button", "dock-move", {
        type: "button", "aria-label": "Move " + labels[node.active] + " panel",
        "aria-expanded": "false", "aria-controls": "dock-options-" + node.active,
      });
      trigger.textContent = "Dock…";
      const options = make("div", "dock-options", { id: "dock-options-" + node.active });
      options.hidden = true;
      function toggleOptions(open) {
        trigger.setAttribute("aria-expanded", String(open));
        options.hidden = !open;
        if (!open) return;
        const bounds = trigger.getBoundingClientRect();
        const available = window.innerHeight - bounds.bottom;
        const below = available >= Math.min(options.scrollHeight + 8, 230) || available >= bounds.top;
        options.style.left = Math.max(4, Math.min(bounds.right - options.offsetWidth, window.innerWidth - options.offsetWidth - 4)) + "px";
        options.style.top = below ? bounds.bottom + "px" : "auto";
        options.style.bottom = below ? "auto" : window.innerHeight - bounds.top + "px";
        options.style.maxHeight = Math.max(0, (below ? available : bounds.top) - 4) + "px";
        options.firstElementChild.focus();
      }
      function addOption(text, action) {
        const button = make("button", "menu-item", { type: "button" });
        button.textContent = text;
        button.addEventListener("click", action);
        options.append(button);
      }
      for (const target of visible) {
        if (node.active === target || hiddenBars.has(panels[target])) continue;
        for (const [title, placement] of [["Left of", "left"], ["Right of", "right"], ["Above", "above"], ["Below", "below"], ["Tab with", "tabs"]]) {
          addOption(title + " " + labels[target], () => move(node.active, target, placement));
        }
      }
      addOption("Reset panel layout", () => {
        layout = defaultLayout();
        detached = false;
        render(node.active);
      });
      trigger.addEventListener("click", () => toggleOptions(options.hidden));
      bar.addEventListener("focusout", event => {
        if (!bar.contains(event.relatedTarget)) toggleOptions(false);
      });
      options.addEventListener("keydown", event => {
        if (event.key !== "Escape") return;
        event.preventDefault();
        toggleOptions(false);
        trigger.focus();
      });
      bar.append(trigger, options);
      return bar;
    }

    function renderSplit(node) {
      const row = node.axis === "rows";
      const box = make("div", "dock-split");
      const grip = make("div", "dock-grip", {
        role: "separator", tabindex: "0", "data-dock-control": "",
        "aria-orientation": row ? "horizontal" : "vertical",
        "aria-label": row ? "Resize panels above and below" : "Resize panels to the left and right",
      });
      box.dataset.axis = node.axis;
      const first = renderNode(node.first);
      box.append(first, grip, renderNode(node.second));
      const firstMin = minimum(node.first, node.axis);
      const secondMin = minimum(node.second, node.axis);
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

    function renderNode(node) {
      if (node.axis) return renderSplit(node);
      const box = make("div", "dock-group");
      box.dataset.dockTarget = node.active;
      box.append(strip(node));
      for (const panel of node.tabs) {
        const content = panels[panel];
        content.hidden = panel !== node.active;
        content.setAttribute("role", "tabpanel");
        content.setAttribute("aria-labelledby", "dock-tab-" + panel);
        box.append(content);
      }
      return box;
    }

    function render(focusPanel) {
      const focused = document.activeElement;
      const shown = new Set([...visible].filter(panel => !hiddenBars.has(panels[panel])));
      const classic = browser === "classic" && !detached && shown.has("inspector") && shown.has("browser");
      const focusedPanel = classicHost.contains(focused) ? "browser" :
        Object.keys(panels).find(panel => panels[panel].contains(focused) || tabButtons.get(panel) === focused);
      for (const area of scrollAreas) {
        if (area.getClientRects().length) scroll.set(area, [area.scrollLeft, area.scrollTop]);
      }
      root.dataset.docking = "";
      tabButtons.clear();
      if (classic) shown.delete("browser");
      classicHost.querySelector(".dock-bar")?.remove();
      classicHost.hidden = !classic;
      if (classic) {
        classicHost.prepend(strip(group(["browser"]), true));
        sampleBrowser.setAttribute("role", "tabpanel");
        sampleBrowser.setAttribute("aria-labelledby", "dock-tab-browser");
      } else {
        sampleBrowser.removeAttribute("role");
        sampleBrowser.removeAttribute("aria-labelledby");
      }
      const host = classic ? classicHost : panels.browser;
      if (sampleBrowser.parentNode !== host) host.append(sampleBrowser);
      const hidden = make("div", "");
      hidden.hidden = true;
      for (const [panel, content] of Object.entries(panels)) {
        if (!shown.has(panel)) {
          content.hidden = true;
          hidden.append(content);
        }
      }
      root.replaceChildren(renderNode(visibleLayout(layout, shown)), hidden, preview);
      for (const [area, [left, top]] of scroll) {
        if (area.getClientRects().length) area.scrollTo(left, top);
      }
      if (focusPanel) tabButtons.get(focusPanel)?.focus({ preventScroll: true });
      else if (focusedPanel && focused.getClientRects().length) focused.focus({ preventScroll: true });
      else if (focusedPanel) {
        const fallback = focusedPanel === "timeline" ? root.querySelector("[role=tab][aria-selected=true]")
          : byId("view-" + focusedPanel);
        (tabButtons.get(focusedPanel) ?? fallback)?.focus({ preventScroll: true });
      }
    }

    function dropAt(event, panel = dragged) {
      const box = event.target.closest("[data-dock-target]");
      if (!box) return null;
      const tab = event.target.closest(".dock-tab");
      const target = tab ? tab.id.slice("dock-tab-".length) : box.dataset.dockTarget;
      if (target === panel) return null;
      const bounds = box.getBoundingClientRect();
      const x = (event.clientX - bounds.left) / bounds.width;
      const y = (event.clientY - bounds.top) / bounds.height;
      const placement = event.target.closest(".dock-bar") ? "tabs"
        : y <= .25 ? "above" : y >= .75 ? "below" : x <= .25 ? "left" : x >= .75 ? "right" : "tabs";
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

    root.addEventListener("dragover", event => {
      if (!dragged) return;
      const destination = dropAt(event);
      preview.hidden = !destination;
      if (!destination) return;
      event.preventDefault();
      event.dataTransfer.dropEffect = "move";
      const { x, y, width, height } = dockPlacement(destination);
      const dock = root.getBoundingClientRect();
      preview.dataset.placement = destination.placement;
      preview.style.cssText = `left:${x - dock.left}px;top:${y - dock.top}px;width:${width}px;height:${height}px`;
    });
    root.addEventListener("dragleave", event => {
      if (dragged && !root.contains(event.relatedTarget)) preview.hidden = true;
    });

    function endDrag() {
      dragged = null;
      preview.hidden = true;
    }
    root.addEventListener("dragend", endDrag);
    root.addEventListener("drop", event => {
      if (!dragged) return;
      const panel = dragged;
      const destination = dropAt(event);
      endDrag();
      if (destination) {
        event.preventDefault();
        move(panel, destination.target, destination.placement);
      }
    });

    const workspace = byId("workspace");
    const editor = byId("layout-editor");
    const ribbon = byId("editor-ribbon");
    const transport = byId("editor-transport");
    const ribbonPicker = byId("layout-ribbon-group");
    const transportPicker = byId("layout-transport");
    const ribbonGroups = () => [...ribbon.children].filter(item => item.matches(".ribbon-group"));
    const initialRibbon = ribbonGroups();
    let draft = null;

    const header = byId("editor-header");
    const status = byId("statusbar");
    const arrange = panels.timeline;
    const overview = arrange.querySelector(".overview");
    const ruler = arrange.querySelector(".ruler");
    const grid = byId("scroll");
    const controller = byId("volume-controller-panel");
    const laneColumn = arrange.querySelector(".ruler-sidebar");
    const defaultFrame = [header, ribbon, root, transport, status];
    const defaultTimeline = [overview, ruler, grid, controller];
    let frameOrder = defaultFrame.slice();
    let timelineOrder = defaultTimeline.slice();

    const barChoices = new Map([
      [header, "Header"], [ribbon, "Ribbon"], [transport, "Transport"], [status, "Status bar"],
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
    const layoutEntry = byId("layout-edit-entry");
    const layoutEntryAnchor = document.createComment("Layout entry");
    layoutEntry.before(layoutEntryAnchor);
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
      const above = frameOrder.indexOf(transport) < frameOrder.indexOf(root);
      workspace.toggleAttribute("data-transport-top", above);
      transportPicker.value = above ? "above" : "below";
      arrange.toggleAttribute("data-layout-lane-sidebar-hidden", hiddenBars.has(laneColumn));
      recovery.hidden = workspace.hidden || !hiddenBars.has(header);
      if (recovery.hidden) layoutEntryAnchor.after(layoutEntry);
      else recovery.append(layoutEntry);
      syncTimelineRows();
      refreshBarPicker();
    }

    function placeBar(bar, target, after) {
      const stack = barStack(bar);
      stack.splice(stack.indexOf(bar), 1);
      stack.splice(stack.indexOf(target) + Number(after), 0, bar);
      applyBarLayout();
    }

    function placeTransport(above) {
      placeBar(transport, root, !above);
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
        ["transport", "Transport", transport],
        ["inspector", "Inspector", byId("editor-inspector")],
        ["browser", "Browser", byId("sample-browser")],
        ["status", "Status bar", byId("statusbar")],
      ];
      for (const [region, label, host, extra] of regions) {
        const candidates = [...host.querySelectorAll("button")];
        if (extra) candidates.push(...byId("editor-arrange").querySelectorAll(extra));
        for (const button of candidates) {
          if (button === layoutEntry || buttons.has(button) ||
              button.closest("#history, #sample-browser-entries, .dock-bar") ||
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

    function layoutDropAt(source, x, y) {
      const under = document.elementFromPoint(x, y);
      if (!under) return null;
      if (source.classList.contains("dock-tab")) {
        const panel = source.id.slice("dock-tab-".length);
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
        (buttons.has(handle) || handle?.classList.contains("dock-tab") ? handle : null);
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
    for (const bar of [header, ribbon, transport, status, overview]) {
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
          detached,
          sizes: snapshotLayoutSizes(),
          waveform: waveformHeight,
          visibleWaveform: visibleWaveformHeight,
          ribbon: ribbonGroups(),
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
        detached = draft.detached;
        ribbon.append(...draft.ribbon);
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
      layoutEntry.focus();
    }

    ribbonPicker.addEventListener("change", refreshRibbonPicker);
    transportPicker.addEventListener("change", () => placeTransport(transportPicker.value === "above"));
    byId("layout-left").addEventListener("click", () => moveRibbonGroup(-1));
    byId("layout-right").addEventListener("click", () => moveRibbonGroup(1));
    byId("layout-cancel").addEventListener("click", () => finishEditing(false));
    byId("layout-keep").addEventListener("click", () => finishEditing(true));
    byId("layout-reset").addEventListener("click", () => {
      layout = defaultLayout();
      detached = false;
      ribbon.append(...initialRibbon);
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

    const update = (nextBrowser, inspector) => {
      if (root.hasAttribute("data-docking") && browser === nextBrowser && inspector === visible.has("inspector")) return;
      const previousBrowser = browser;
      browser = nextBrowser;
      for (const [panel, shown] of [["inspector", inspector], ["browser", browser !== "off"]]) {
        if (shown && !visible.has(panel)) activate(panel);
        if (shown) visible.add(panel);
        else visible.delete(panel);
      }
      if (previousBrowser !== browser && browser === "classic" && detached) activate("browser");
      render();
    };
    update.editLayout = editLayout;
    return update;
  }

  window.editorDocking = { defaultLayout, movePanel, visibleLayout, attach };
})();

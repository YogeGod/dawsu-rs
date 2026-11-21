// Panel visibility, folding rails, and coordination with docking.js.
import { panelSwitches, railSections } from "./helpers.js";

export function attachPanels(ctx) {
  const { env, state, dom } = ctx;
  const { $ } = dom;

  // The docking state: narrow windows fold the Browser for the session (docking.js, from matchMedia).
  function dockState() {
    return ctx.dock?.state?.() ?? { narrow: false, override: null };
  }

  function setDockOverride(value) {
    ctx.dock?.setOverride?.(value);
    ctx.renderPanelLayout();
  }

  function panelPrefs() {
    const panels = state.view.panels ?? {};
    return {
      browserOpen: !!panels.browser,
      inspectorOpen: !!panels.inspector,
      controllerOpen: !!panels.controller,
      classic: !!state.view.classicBrowser,
    };
  }

  // What the frame shows for the Browser ("open", "classic", "folded" or "off") and the Inspector.
  function effectivePanelState() {
    const { narrow, override } = ctx.dockState();
    const prefs = ctx.panelPrefs();
    return env.window.editorDocking.effectivePanels(prefs, narrow, override);
  }

  // Whether a panel shows open: switches, menu rows and View settings show this, not the preference.
  function panelOn(panel) {
    if (panel === "controller") return !!state.view.panels?.controller;
    const current = ctx.effectivePanelState()[panel];
    return current === "open" || current === "classic";
  }

  // Every switch, rail, menu row, checkbox and close button folds or opens a panel through here.
  function togglePanel(panel) {
    if (!state.scene || !state.view.panels || state.view.busy) return;
    const { narrow, override } = ctx.dockState();
    const plan = env.window.editorDocking.togglePlan(panel, ctx.panelPrefs(), narrow, override);
    if (plan.act) return ctx.act(plan.act);
    ctx.setDockOverride(plan.override);
  }

  // A rail icon: open the panel, then bring its section into view.
  async function openPanelAt(panel, section) {
    if (!state.scene || !state.view.panels || state.view.busy) return;
    if (!ctx.panelOn(panel)) await ctx.togglePanel(panel);
    if (!ctx.panelOn(panel)) return;
    let target;
    if (panel === "browser") {
      // The Browser parts write each model at once, so the tab and search field exist right after.
      if (section !== "search") ctx.sampleBrowserAction("source", section === "library" ? "library" : "mapset");
      target = section === "search" ? $("sample-browser-filter")
        : state.browserSource === "library" ? $("sample-browser-library") : $("sample-browser-mapset");
    } else {
      target = $("editor-inspector").querySelector(`section[aria-label="${section}"] button`);
    }
    target?.scrollIntoView?.({ block: "nearest" });
    target?.focus();
  }

  function renderPanelLayout() {
    if (!state.view.panels) return;
    const { controller, laneNames } = state.view.panels;
    const panels = ctx.effectivePanelState();
    for (const [panel, id] of Object.entries(panelSwitches)) $(id).ariaPressed = String(!!state.scene && ctx.panelOn(panel));
    $("editor-arrange").dataset.controller = String(controller);
    $("editor-arrange").dataset.laneNames = String(laneNames);
    // Folded shows the bottom panel's 34 px strip; the tab is kept in the bundle, never saved.
    $("volume-controller-panel").dataset.bottom = controller ? ctx.ui()?.bottomTab?.() ?? "volume" : "folded";
    if (ctx.dock && state.scene) {
      ctx.dock({
        browserStyle: state.view.classicBrowser ? "classic" : "tree",
        browserOpen: !!state.view.panels.browser,
        inspectorOpen: !!state.view.panels.inspector,
      });
    }
    if (state.scene && (panels.browser === "open" || panels.browser === "classic")) ctx.renderSampleBrowser();
  }

  function bindPanels() {
    for (const [panel, id] of Object.entries(panelSwitches)) $(id).addEventListener("click", () => ctx.togglePanel(panel));
    $("browser-fold").addEventListener("click", () => ctx.togglePanel("browser"));
    $("browser-unfold").addEventListener("click", () => ctx.togglePanel("browser"));
    $("inspector-fold").addEventListener("click", () => ctx.togglePanel("inspector"));
    $("inspector-unfold").addEventListener("click", () => ctx.togglePanel("inspector"));
    for (const [id, [panel, section]] of Object.entries(railSections)) {
      $(id).addEventListener("click", () => ctx.openPanelAt(panel, section));
    }
  }

  Object.assign(ctx, {
    dockState, setDockOverride, panelPrefs, effectivePanelState,
    panelOn, togglePanel, openPanelAt, renderPanelLayout, bindPanels,
  });
}

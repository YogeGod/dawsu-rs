// A small fake DOM for docking.js's attach(): elements with attributes, dataset, children, hidden,
// focus and simple selectors (tag, #id, .class, [attr], :scope > child). Not a browser: no layout.

const kebab = name => name.replace(/[A-Z]/g, letter => "-" + letter.toLowerCase());

function matchesCompound(element, compound) {
  if (!element.tagName) return false;
  for (const part of compound.match(/[#.]?[\w-]+|\[[^\]]+\]/g) ?? []) {
    if (part.startsWith("#") && element.id !== part.slice(1)) return false;
    if (part.startsWith(".") && !element.classList.contains(part.slice(1))) return false;
    if (part.startsWith("[")) {
      const [name, value] = part.slice(1, -1).split("=");
      if (!element.hasAttribute(name)) return false;
      if (value !== undefined && element.getAttribute(name) !== value.replace(/"/g, "")) return false;
    }
    if (/^[a-z]/.test(part) && element.tagName !== part.toUpperCase()) return false;
  }
  return true;
}

export function fakeDocument({ narrow = false } = {}) {
  const ids = new Map();
  const listeners = new Map();
  let active = null;
  const queries = [];

  class Node {
    constructor() { this.parentNode = null; this.childNodes = []; }
    get children() { return this.childNodes.filter(node => node.tagName); }
    get firstElementChild() { return this.children[0] ?? null; }
    detach() {
      if (this.parentNode) this.parentNode.childNodes = this.parentNode.childNodes.filter(node => node !== this);
      this.parentNode = null;
    }
    insert(nodes, at) {
      for (const node of nodes) node.detach();
      this.childNodes.splice(at, 0, ...nodes);
      for (const node of nodes) node.parentNode = this;
    }
    append(...nodes) { this.insert(nodes, this.childNodes.length); }
    prepend(...nodes) { this.insert(nodes, 0); }
    replaceChildren(...nodes) {
      for (const node of this.childNodes) node.parentNode = null;
      this.childNodes = [];
      this.append(...nodes);
    }
    before(...nodes) { const parent = this.parentNode; parent.insert(nodes, parent.childNodes.indexOf(this)); }
    after(...nodes) { const parent = this.parentNode; parent.insert(nodes, parent.childNodes.indexOf(this) + 1); }
    remove() { this.detach(); }
    contains(node) {
      for (let at = node; at; at = at.parentNode) if (at === this) return true;
      return false;
    }
  }

  class Element extends Node {
    constructor(tag) {
      super();
      this.tagName = tag.toUpperCase();
      this.attributes = new Map();
      this.hidden = false;
      this.listeners = new Map();
      this.scrollLeft = 0;
      this.scrollTop = 0;
      const self = this;
      this.style = {
        values: new Map(),
        setProperty(name, value) { this.values.set(name, value); },
        removeProperty(name) { this.values.delete(name); },
        getPropertyValue(name) { return this.values.get(name) ?? ""; },
      };
      this.dataset = new Proxy({}, {
        get: (_, key) => self.getAttribute("data-" + kebab(String(key))) ?? undefined,
        set: (_, key, value) => { self.setAttribute("data-" + kebab(String(key)), value); return true; },
        deleteProperty: (_, key) => { self.removeAttribute("data-" + kebab(String(key))); return true; },
      });
      this.classList = {
        contains: name => self.className.split(/\s+/).includes(name),
        add: name => { if (!self.classList.contains(name)) self.className = (self.className + " " + name).trim(); },
        toggle() {},
      };
    }
    get id() { return this.getAttribute("id") ?? ""; }
    set id(value) { this.setAttribute("id", value); }
    get className() { return this.getAttribute("class") ?? ""; }
    set className(value) { this.setAttribute("class", value); }
    setAttribute(name, value) {
      this.attributes.set(name, String(value));
      if (name === "id") ids.set(String(value), this);
    }
    getAttribute(name) { return this.attributes.has(name) ? this.attributes.get(name) : null; }
    hasAttribute(name) { return this.attributes.has(name); }
    removeAttribute(name) { this.attributes.delete(name); }
    toggleAttribute(name, on = !this.hasAttribute(name)) {
      if (on) this.attributes.set(name, "");
      else this.attributes.delete(name);
      return on;
    }
    addEventListener(name, callback) { this.listeners.set(name, callback); }
    dispatch(name, event = {}) { this.listeners.get(name)?.({ target: this, ...event }); }
    matches(selector) { return selector.split(",").some(one => matchesCompound(this, one.trim())); }
    closest(selector) {
      for (let at = this; at?.tagName; at = at.parentNode) if (at.matches(selector)) return at;
      return null;
    }
    querySelectorAll(selector) {
      queries.push(selector);
      if (selector.startsWith(":scope > ")) return this.children.filter(child => child.matches(selector.slice(9)));
      const found = [];
      const walk = node => { for (const child of node.children) { if (child.matches(selector)) found.push(child); walk(child); } };
      walk(this);
      return found;
    }
    querySelector(selector) { return this.querySelectorAll(selector)[0] ?? null; }
    getBoundingClientRect() { return { left: 0, top: 0, right: 0, bottom: 0, width: 0, height: 0, x: 0, y: 0 }; }
    // Shown when it and every ancestor up to the document body are not hidden.
    getClientRects() {
      for (let at = this; at; at = at.parentNode) {
        if (at.hidden) return [];
        if (at === document.body) return [{}];
      }
      return [];
    }
    scrollTo() {}
    focus() { active = this; }
    get selectedOptions() { return [this.children[this.selectedIndex ?? 0]].filter(Boolean); }
  }

  const document = {
    body: new Element("body"),
    get activeElement() { return active ?? document.body; },
    getElementById: id => ids.get(id) ?? null,
    createElement: tag => new Element(tag),
    createComment: () => new Node(),
    addEventListener(name, callback) { listeners.set(name, callback); },
  };
  // Build markup from [tag, attributes, ...children] arrays.
  const build = ([tag, attrs = {}, ...children]) => {
    const element = new Element(tag);
    for (const [name, value] of Object.entries(attrs)) {
      if (name === "hidden") element.hidden = !!value;
      else element.setAttribute(name, value);
    }
    element.append(...children.map(build));
    return element;
  };
  const mediaListeners = [];
  const media = {
    matches: narrow,
    addEventListener(_, callback) { mediaListeners.push(callback); },
  };
  const window = {
    matchMedia: () => media,
    addEventListener() {},
    setNarrow(on) { media.matches = on; for (const callback of mediaListeners) callback(); },
  };
  return { document, window, build, Element, MutationObserver: class { observe() {} } };
}

// The parts of index.html docking.js reads, in their real nesting.
export function editorPage(build) {
  const button = id => ["button", { id }];
  return build(["body", {},
    ["header", { id: "editor-header" }, button("menu-view"), button("view-browser"), button("view-inspector")],
    ["section", { id: "workspace" },
      ["section", { id: "layout-editor", hidden: true },
        ["select", { id: "layout-ribbon-group" }], ["select", { id: "layout-bar" }],
        ["input", { id: "layout-bar-hidden" }], button("layout-bar-earlier"), button("layout-bar-later"),
        ["select", { id: "layout-button" }], ["select", { id: "layout-button-location" }],
        button("layout-button-before"), button("layout-button-after"), ["input", { id: "layout-button-hidden" }],
        ["input", { id: "layout-button-width" }], ["input", { id: "layout-button-height" }],
        button("layout-left"), button("layout-right"), button("layout-reset"), button("layout-cancel"), button("layout-keep")],
      ["div", { id: "editor-ribbon" }, ["div", { class: "ribbon-group", "aria-label": "Map" }]],
      ["main", { id: "editor-dock" },
        ["aside", { id: "tree-browser-host", hidden: true },
          ["div", { class: "panel-head" }, ["h2"], button("browser-fold")],
          ["nav", { class: "panel-rail", hidden: true }, button("browser-unfold")],
          ["section", { id: "sample-browser" }, ["div", { id: "sample-browser-entries" }], button("sample-browser-close")]],
        ["div", { id: "editor-arrange" },
          ["div", { class: "ruler" }, ["div", { class: "ruler-sidebar" }]],
          ["div", { id: "scroll" }, ["div", { id: "lanes" }], ["canvas", { id: "grid" }]],
          ["div", { id: "volume-controller-panel" }],
          ["div", { class: "overview" }]],
        ["aside", { id: "editor-inspector" },
          ["div", { class: "panel-head" }, ["h2"], button("inspector-fold")],
          ["nav", { class: "panel-rail", hidden: true }, button("inspector-unfold")],
          ["section", { "aria-label": "Selection" }],
          ["div", { id: "classic-browser-host", hidden: true }]]]],
    ["div", { id: "statusbar" }],
    button("layout-edit-entry")]);
}

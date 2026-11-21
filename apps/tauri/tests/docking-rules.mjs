// docking.js's pure rules (which panels show, what a fold does) for vm runs of app.js, which always
// has docking.js loaded first in the real page. attach() is left out: these fakes have no layout.
import { readFile } from "node:fs/promises";
import { createContext, runInContext } from "node:vm";

const source = await readFile(new URL("../dist/docking.js", import.meta.url), "utf8");
const context = createContext({ window: {} });
runInContext(source, context);
const { effectivePanels, togglePlan } = context.window.editorDocking;
export const dockingRules = { effectivePanels, togglePlan, attach: () => undefined };

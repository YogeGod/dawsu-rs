import { folderDragType } from "./lanes.js";

// The lane and folder headers' event handlers. Each sends the action app.js's laneHeaderAction()
// runs; send and tick come in so tests can drive them with fake events.
export function laneHandlers(send, tick) {
  return {
    // The row outside its buttons selects the lane like its name does.
    rowClick(event, row) {
      if (!event.target.closest?.("button")) send("select", row, event);
    },
    select: (event, row) => send("select", row, event),
    mix: (row, operation) => send("mix", row, operation),
    family: row => send("family", row),
    addLayer: row => send("addLayer", row),
    remove: row => send("remove", row),
    fold: set => send("folder", set),
    // The folder name folds too, as the old header's name button did.
    foldName: set => send("folder", set),
    folderMix: (set, operation) => send("folderMix", set, operation),

    // Alt+Up and Alt+Down move a folder; focus goes back to its strip once the rows have moved.
    async folderKey(event, set) {
      if (!event.altKey || !["ArrowUp", "ArrowDown"].includes(event.key)) return;
      event.preventDefault();
      event.stopPropagation();
      const header = event.currentTarget.closest(".sample-folder");
      send("folderStep", set, event.key === "ArrowDown");
      await tick();
      header?.querySelector(".fold")?.focus();
    },

    dragStart(event, set) {
      event.dataTransfer.setData(folderDragType, set);
      event.dataTransfer.effectAllowed = "move";
    },
    dragOver(event) {
      if ([...event.dataTransfer.types].includes(folderDragType)) event.preventDefault();
    },
    drop(event, set) {
      const from = event.dataTransfer.getData(folderDragType);
      if (!from) return;
      event.preventDefault();
      const box = event.currentTarget.getBoundingClientRect();
      send("folderMove", from, set, event.clientY > box.y + box.height / 2);
    },
  };
}

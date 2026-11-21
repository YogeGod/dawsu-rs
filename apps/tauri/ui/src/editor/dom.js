// Lightweight DOM creation and lookup helpers bound to the active document.

export function createDom(env) {
  const $ = (id) => env.document.getElementById(id);

  function makeButton(label, click) {
    const button = env.document.createElement("button");
    button.textContent = label;
    button.addEventListener("click", click);
    return button;
  }

  function paragraph(value) {
    const node = env.document.createElement("p");
    node.textContent = value;
    return node;
  }

  function input(type, value, min, max, step = 1) {
    const field = env.document.createElement("input");
    field.type = type;
    field.value = value;
    field.min = min;
    field.max = max;
    field.step = step;
    return field;
  }

  function labeled(label, control) {
    const node = env.document.createElement("label");
    node.append(label, control);
    return node;
  }

  function setValue(id, value) {
    if (env.document.activeElement !== $(id)) $(id).value = value;
  }

  function number(id, fallback) {
    return $(id).valueAsNumber || ($(id).value === "0" ? 0 : fallback);
  }

  return { $, makeButton, paragraph, input, labeled, setValue, number };
}

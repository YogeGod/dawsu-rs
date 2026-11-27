import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { render } from "svelte/server";
import { bufferError, canApply } from "../src/lib/audioPrefs.js";
import AudioTab from "../src/parts/preferences/AudioTab.svelte";

const ready = { locked: false, loadError: "", backend: "system", device: "", bufferMs: "" };

test("bufferError checks each keystroke against the backend's range, and empty means device default", () => {
  assert.equal(bufferError("system", "1"), "Buffer must be between 2 and 500 ms.");
  assert.equal(bufferError("system", "10"), "");
  assert.equal(bufferError("system", 2), "");
  assert.equal(bufferError("system", 500), "");
  assert.equal(bufferError("system", 501), "Buffer must be between 2 and 500 ms.");
  assert.equal(bufferError("system", "2.5"), "Buffer must be between 2 and 500 ms.");
  for (const invalid of [0, -1, "   ", NaN, Infinity]) {
    assert.equal(bufferError("system", invalid), "Buffer must be between 2 and 500 ms.");
  }
  assert.equal(bufferError("mme", "5"), "Buffer must be between 10 and 500 ms.");
  assert.equal(bufferError("direct_sound", 9), "Buffer must be between 10 and 500 ms.");
  assert.equal(bufferError("asio", "2"), "");
  for (const empty of ["", null, undefined]) assert.equal(bufferError("system", empty), "");
});

test("canApply: a lock, a load error, a bad buffer or ASIO without a driver turn Apply off", () => {
  assert.equal(canApply(ready), true);
  assert.equal(canApply({ ...ready, locked: true }), false);
  assert.equal(canApply({ ...ready, loadError: "No devices" }), false);
  assert.equal(canApply({ ...ready, bufferMs: "1" }), false);
  assert.equal(canApply({ ...ready, bufferMs: "10" }), true);
  for (const device of ["", "   ", null, undefined]) {
    assert.equal(canApply({ ...ready, backend: "asio", device }), false, `asio with ${JSON.stringify(device)}`);
  }
  assert.equal(canApply({ ...ready, backend: "asio", device: "asio:focusrite" }), true);
});

const config = {
  backend: "system", available: [{ id: "system", label: "WASAPI" }, { id: "asio", label: "ASIO" }],
  devices: [{ id: "wasapi:speakers", name: "Speakers" }], current: "Speakers",
};
function tab(props) {
  return render(AudioTab, { props: {
    audioConfig: config, selectedBackend: "system", selectedDevice: "", bufferMs: "", loadError: "", applyError: "",
    audioLocked: false, view: {}, onloadaudio() {}, onapplyaudio() {}, onsendaction() {}, onaudioedit() {}, ...props,
  } }).body;
}
const applyButton = html => html.match(/<button[^>]*>Apply output<\/button>/)?.[0] ?? "";
// Backend, then Device, then the Playback selects.
const deviceSelect = html => [...html.matchAll(/<select[^>]*>/g)][1]?.[0] ?? "";

test("AudioTab: a bad buffer shows its error and turns off only Apply; a good one clears it", () => {
  const bad = tab({ bufferMs: "1" });
  assert.match(bad, /Buffer must be between 2 and 500 ms\./);
  assert.match(applyButton(bad), /\sdisabled/);
  assert.doesNotMatch(deviceSelect(bad), /\sdisabled/);
  const good = tab({ bufferMs: "10" });
  assert.doesNotMatch(good, /Buffer must be between/);
  assert.doesNotMatch(applyButton(good), /\sdisabled/);
});

test("AudioTab: an apply error is shown but disables nothing; a load error disables the Device select", () => {
  const failed = tab({ applyError: "The device is in use." });
  assert.match(failed, /The device is in use\./);
  assert.doesNotMatch(applyButton(failed), /\sdisabled/);
  assert.doesNotMatch(deviceSelect(failed), /\sdisabled/);
  const unloaded = tab({ loadError: "No output devices." });
  assert.match(unloaded, /No output devices\./);
  assert.match(deviceSelect(unloaded), /\sdisabled/);
});

test("AudioTab: ASIO with no driver keeps Apply off and says to choose one, unless loadError is active", () => {
  const none = tab({ selectedBackend: "asio", selectedDevice: "" });
  assert.match(none, /Choose an ASIO driver first\./);
  assert.match(applyButton(none), /\sdisabled/);
  const unavailable = tab({ selectedBackend: "asio", selectedDevice: "", loadError: "ASIO is unavailable in this build." });
  assert.match(unavailable, /ASIO is unavailable in this build\./);
  assert.match(deviceSelect(unavailable), /\sdisabled/);
  assert.match(applyButton(unavailable), /\sdisabled/);
  assert.doesNotMatch(unavailable, /Choose an ASIO driver first\./);
  const picked = tab({ selectedBackend: "asio", selectedDevice: "asio:focusrite" });
  assert.doesNotMatch(picked, /Choose an ASIO driver first\./);
  assert.doesNotMatch(applyButton(picked), /\sdisabled/);
});

test("PreferencesModal clears applyError on loadAudio and passes onaudioedit to AudioTab", () => {
  const source = readFileSync(new URL("../src/parts/PreferencesModal.svelte", import.meta.url), "utf8");
  assert.match(source, /async function loadAudio\([^)]*\)\s*\{\s*applyError = "";/);
  assert.match(source, /onaudioedit=\{\(\) => \(applyError = ""\)\}/);
});

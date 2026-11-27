import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { render } from "svelte/server";
import GeneralTab from "../src/parts/preferences/GeneralTab.svelte";

const props = view => ({
  scales: [100],
  scale: 100,
  preset: "DAW",
  view,
  busy: false,
  onapplyscale() {},
  onpickpreset() {},
  onpicksongs() {},
  onpicklibrary() {},
  onsendaction() {},
});

const intervalSelect = html => html.match(/<select[^>]*aria-label="Recovery copy interval"[^>]*>([\s\S]*?)<\/select>/)?.[1] ?? "";

test("GeneralTab offers the five autosave intervals under Save & Export with the active one selected", () => {
  const html = render(GeneralTab, { props: props({ autosaveIntervalSecs: 60 }) }).body;
  const options = [...intervalSelect(html).matchAll(/<option([^>]*)>([^<]*)<\/option>/g)];
  assert.deepEqual(options.map(([, attrs]) => attrs.match(/value="(\d+)"/)?.[1]), ["0", "15", "30", "60", "120"]);
  assert.deepEqual(options.map(([, , label]) => label), ["Off", "15s", "30s (default)", "60s", "120s"]);
  assert.deepEqual(options.filter(([, attrs]) => /\sselected/.test(attrs)).map(([, , label]) => label), ["60s"]);
  assert.ok(html.indexOf("Save &amp; Export") < html.indexOf("Recovery copy interval"), "inside Save & Export");
  assert.match(html, /<label class="pref-field">/);

  const fallback = render(GeneralTab, { props: props({}) }).body;
  assert.deepEqual([...intervalSelect(fallback).matchAll(/<option[^>]*\sselected[^>]*>([^<]*)</g)].map(m => m[1]), ["30s (default)"]);
});

test("GeneralTab sends the picked interval as a number", () => {
  const source = readFileSync(new URL("../src/parts/preferences/GeneralTab.svelte", import.meta.url), "utf8");
  assert.match(source, /onsendaction\(\{ kind: "autosave_interval", secs: Number\(e\.currentTarget\.value\) \}\)/);
});

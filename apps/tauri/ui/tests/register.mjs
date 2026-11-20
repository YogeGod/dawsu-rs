// npm test: lets node:test import .svelte and .svelte.js files, compiled for svelte/server.
import { register } from "node:module";

register("./svelte-hooks.mjs", import.meta.url);

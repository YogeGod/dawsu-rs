<script>
  import { RECIPE_OPTIONS } from "../../lib/node-graph.js";

  let {
    view,
    recipeChoice = "classic",
    mode = "simple",
    error = "",
    onloadrecipe,
    onpickopen,
    onpicksave,
    onsetmode,
    onshowineditor,
    onclose,
  } = $props();
</script>

<header class="ng-header">
  <div class="ng-brand">
    <span class="ng-diamond" aria-hidden="true"></span>
    <span>Node Graph</span>
  </div>

  {#if view?.hasDocument}
    <span class="ng-map-chip">{view.mapTitle || "Open map"}{view.mapVersion ? " [" + view.mapVersion + "]" : ""}</span>
  {:else}
    <span class="ng-map-chip">No difficulty open</span>
  {/if}

  <div class="ng-recipe-box">
    <span>Recipe:</span>
    <select aria-label="Recipe" value={recipeChoice} onchange={(e) => onloadrecipe(e.currentTarget.value)}>
      {#each RECIPE_OPTIONS as opt (opt.id)}
        <option value={opt.id}>{opt.label}</option>
      {/each}
    </select>
  </div>

  <button type="button" class="ng-btn" onclick={onpickopen}>Load…</button>
  <button type="button" class="ng-btn" onclick={onpicksave}>Save…</button>

  <div class="ng-spacer"></div>

  <div class="ng-seg" role="group" aria-label="View mode">
    <button type="button" class="ng-seg-btn" aria-pressed={mode === "simple"} onclick={() => onsetmode("simple")}>
      Simple Flow
    </button>
    <button type="button" class="ng-seg-btn" aria-pressed={mode === "raw"} onclick={() => onsetmode("raw")}>
      Raw Blocks ({view?.nodes?.length ?? 0})
    </button>
  </div>

  <button type="button" class="ng-btn ng-btn-teal" onclick={onshowineditor}>Show in Editor</button>
  <button type="button" class="ng-btn" onclick={onclose}>Close</button>
</header>

{#if error || view?.error}
  <div class="ng-banner-err">{error || view.error}</div>
{/if}

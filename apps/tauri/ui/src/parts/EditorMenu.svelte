<script>
  import { bridge } from "../lib/bridge.svelte.js";

  // menus.js builds the rows from the command list (lib/commands.js), works out what is enabled and
  // checked, and runs a row when it is pressed. The classes are the ones style.css already styles.
  const rows = $derived(bridge.menu?.rows ?? []);
  const press = index => bridge.callAction("menuAction", index);
</script>

{#snippet button(row)}
  <button type="button" tabindex="-1" class={row.className} role={row.role}
    aria-checked={row.checked === undefined ? undefined : String(row.checked)} disabled={row.disabled}
    title={row.role === "menuitemradio" ? row.label : undefined}
    onclick={() => press(row.index)}
  >{#if row.icon}<img class="button-icon" src={"studio-assets/icons/" + row.icon + ".svg"} alt="">{/if}{row.label}{#if row.chip}<kbd hidden={!row.keys}>{row.keys}</kbd>{/if}{#if row.switch}<span class="menu-switch" aria-hidden="true"></span>{/if}</button>
{/snippet}

{#each rows as row}
  {#if row.type === "separator"}
    <div class="menu-separator" role="separator"></div>
  {:else if row.type === "choices"}
    <div class="menu-choice-group" role="group" aria-label={row.label}>
      <div class="menu-choice-label">{row.label}</div>
      <div class="menu-choices" class:menu-choices-list={row.label === "Difficulty" || row.choices.length > 3 || row.choices.some(c => c.label.length > 14)}>
        {#each row.choices as choice}{@render button(choice)}{/each}
      </div>
    </div>
  {:else}
    {@render button(row.item)}
  {/if}
{/each}

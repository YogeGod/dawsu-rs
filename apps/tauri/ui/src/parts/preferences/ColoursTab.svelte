<script>
  import {
    COLOR_ROWS, SCHEMES, hexToHsb, hsbToHex, resolvePalette, rowStartHsb,
  } from "../../lib/theme.js";

  const COLOR_GROUPS = ["Custom Colors", "Grid Colors", "Ruler Colors"];

  let {
    colorConfig,
    expandedColorRow = $bindable(null),
    onupdate,
  } = $props();

  function pickScheme(schemeId) {
    onupdate({ ...colorConfig, scheme: schemeId });
  }

  function toggleColorRow(row, enabled) {
    const rows = { ...(colorConfig.rows ?? {}) };
    if (enabled) {
      rows[row.id] = rowStartHsb(row, colorConfig.scheme);
    } else {
      delete rows[row.id];
      for (const child of COLOR_ROWS) {
        if (child.requires === row.id) delete rows[child.id];
      }
      if (expandedColorRow === row.id) expandedColorRow = null;
    }
    onupdate({ ...colorConfig, rows });
  }

  function setColorRowChannel(row, channel, value) {
    const rows = { ...(colorConfig.rows ?? {}) };
    const current = rows[row.id] ?? rowStartHsb(row, colorConfig.scheme);
    rows[row.id] = { ...current, [channel]: value };
    onupdate({ ...colorConfig, rows });
  }

  function shownRowHex(row) {
    const hsb = colorConfig.rows?.[row.id];
    if (!hsb) return null;
    if (row.noBrightness) return hsbToHex({ ...hsb, b: rowStartHsb(row, colorConfig.scheme).b });
    if (row.hasHue) return hsbToHex(hsb);
    const schemeHue = colorConfig.rows?.scheme?.h ?? hexToHsb(resolvePalette({ scheme: colorConfig.scheme, rows: {} }).panel).h;
    return hsbToHex({ h: schemeHue, s: hsb.s, b: hsb.b });
  }
</script>

<div class="pref-group">
  <span class="pref-label">Default Color Schemes</span>
  <div class="colour-schemes" role="group" aria-label="Default Color Schemes">
    {#each SCHEMES as scheme (scheme.id)}
      <button type="button" class="scheme-swatch" class:active={colorConfig.scheme === scheme.id}
        title={scheme.name} aria-label={scheme.name + " colour scheme"} aria-pressed={colorConfig.scheme === scheme.id}
        onclick={() => pickScheme(scheme.id)}>
        <span class="scheme-chip" style:background={scheme.swatch}></span>
      </button>
    {/each}
  </div>
  <p class="pref-caption">Changes show immediately across the editor and timeline. Press Defaults to restore Obsidian.</p>
</div>

{#each COLOR_GROUPS as group (group)}
  <div class="pref-group colour-group">
    <span class="pref-label">{group}</span>
    {#each COLOR_ROWS.filter(r => r.group === group) as row (row.id)}
      {@const enabled = Boolean(colorConfig.rows?.[row.id])}
      {@const parentMissing = Boolean(row.requires && !colorConfig.rows?.[row.requires])}
      {@const current = colorConfig.rows?.[row.id] ?? rowStartHsb(row, colorConfig.scheme)}
      {@const hex = shownRowHex(row)}
      <div class="colour-row">
        <label class="pref-check colour-check">
          <input type="checkbox" checked={enabled} disabled={parentMissing}
            onchange={(e) => toggleColorRow(row, e.currentTarget.checked)} />
          <span>{row.label}</span>
        </label>
        <button type="button" class="colour-swatch-btn" disabled={!enabled}
          aria-label={row.group + " " + row.label + " colour"}
          aria-expanded={expandedColorRow === row.id}
          onclick={() => (expandedColorRow = expandedColorRow === row.id ? null : row.id)}>
          <span class="colour-swatch-fill" style:background={hex ?? "transparent"}></span>
          <span class="colour-swatch-caret" aria-hidden="true">▾</span>
        </button>
      </div>
      {#if enabled && expandedColorRow === row.id}
        <div class="colour-sliders">
          {#if row.hasHue}
            <label class="colour-slider-row">
              <span>Hue</span>
              <input type="range" min="0" max="360" step="1" value={Math.round(current.h)}
                oninput={(e) => setColorRowChannel(row, "h", Number(e.currentTarget.value))} />
              <span class="colour-val">{Math.round(current.h)}</span>
            </label>
          {/if}
          <label class="colour-slider-row">
            <span>Saturation</span>
            <input type="range" min="0" max="100" step="1" value={Math.round(current.s * 100)}
              oninput={(e) => setColorRowChannel(row, "s", Number(e.currentTarget.value) / 100)} />
            <span class="colour-val">{Math.round(current.s * 100)}</span>
          </label>
          {#if !row.noBrightness}
            <label class="colour-slider-row">
              <span>Brightness</span>
              <input type="range" min="0" max="100" step="1" value={Math.round(current.b * 100)}
                oninput={(e) => setColorRowChannel(row, "b", Number(e.currentTarget.value) / 100)} />
              <span class="colour-val">{Math.round(current.b * 100)}</span>
            </label>
          {/if}
        </div>
      {/if}
    {/each}
  </div>
{/each}

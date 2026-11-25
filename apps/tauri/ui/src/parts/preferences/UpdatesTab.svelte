<script>
  let {
    view,
    updateChecking = false,
    updateDownloading = false,
    updateStatus = "",
    updateResult = null,
    updateError = "",
    updateCopied = false,
    oncheck,
    ondownload,
    oncopyurl,
    onsendaction,
  } = $props();
</script>

<div class="pref-group">
  <div class="pref-row-head">
    <span class="pref-label">dawsu! Desktop</span>
    <button type="button" disabled={updateChecking} onclick={oncheck}>
      {updateChecking ? "Checking…" : "Check for updates"}
    </button>
  </div>
  <label class="pref-check">
    <input type="checkbox" checked={view.autoCheckUpdates !== false}
      onchange={(e) => onsendaction({ kind: "auto_check_updates", on: e.currentTarget.checked })} />
    <span>Check GitHub Releases automatically when dawsu! starts</span>
  </label>
  <p class="pref-caption">
    Always verifies downloaded installers and portable ZIP archives against <code>SHA256SUMS.txt</code> before installing on exit.
  </p>
  {#if updateError}
    <p class="pref-error">{updateError}</p>
  {:else if updateResult}
    {#if updateResult.updateAvailable}
      <p class="pref-label">Update available: v{updateResult.latestVersion}</p>
      <p class="pref-caption">Installed version: v{updateResult.currentVersion}</p>
      {#if (updateResult.installerUrl || updateResult.portableUrl) && updateResult.sha256sumsUrl}
        <div class="pref-row-head">
          <button type="button" class="pref-primary" disabled={updateDownloading} onclick={ondownload}>
            {updateDownloading ? "Downloading…" : `Update to v${updateResult.latestVersion} and restart`}
          </button>
        </div>
        <p class="pref-caption">
          {updateResult.isInstalled
            ? "Downloads the installer, checks its SHA-256 hash, asks about unsaved work, and installs on close."
            : "Downloads the portable ZIP, checks its SHA-256 hash and flat archive safety, and swaps files on close."}
        </p>
      {/if}
    {:else}
      <p class="pref-caption">You are up to date (v{updateResult.currentVersion}; latest release v{updateResult.latestVersion}).</p>
    {/if}
    {#if updateStatus}
      <p class="pref-caption">{updateStatus}</p>
    {/if}
    {#if updateResult.notes}
      <p class="pref-caption">{updateResult.notes}</p>
    {/if}
    {#if updateResult.releaseUrl}
      <div class="pref-path-row">
        <input type="text" readonly value={updateResult.releaseUrl} aria-label="Release page URL" />
        <button type="button" onclick={oncopyurl}>{updateCopied ? "Copied" : "Copy link"}</button>
      </div>
    {/if}
  {/if}
</div>

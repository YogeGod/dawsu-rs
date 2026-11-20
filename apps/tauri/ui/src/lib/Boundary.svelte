<script>
  let { part: Part, host } = $props();

  // Without the ready flag app.js reports the part as missing in the status bar.
  function failed(error) {
    console.error(error);
    delete host.dataset.uiReady;
  }
</script>

<svelte:boundary onerror={failed}>
  <Part />
</svelte:boundary>

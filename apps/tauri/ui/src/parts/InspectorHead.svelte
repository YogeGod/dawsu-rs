<script>
  import ChevronDown from "../icons/ChevronDown.svg?raw";

  // A section's fold header. Folding hides the body but keeps it in the page.
  let { title, body, open = $bindable(true) } = $props();
</script>

<h2 class="insp-title">
  <button type="button" class="insp-head" aria-expanded={open} aria-controls={body} onclick={() => (open = !open)}>
    <span class="insp-chevron" class:insp-folded={!open}>{@html ChevronDown}</span>{title}
  </button>
</h2>

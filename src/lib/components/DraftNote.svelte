<script lang="ts">
  // A quiet note on a form that was filled from what the owner typed before leaving the screen, and that
  // still differs from what is saved. Discard puts the saved values back.
  let { ondiscard, text = "Unsaved changes" }: { ondiscard: () => void; text?: string } = $props();
</script>

<p class="draft-note small" role="status">
  <span>{text}</span>
  <button class="btn small" type="button" onclick={ondiscard}>Discard</button>
</p>

<style>
  .draft-note { display: flex; align-items: center; gap: 0.6rem; flex-wrap: wrap; margin: 0; color: var(--muted); }
  .draft-note .btn { min-height: 1.9rem; padding: 0 0.65rem; }
</style>

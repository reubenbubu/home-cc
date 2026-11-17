<script lang="ts">
  // Shown for as long as status checks can't work at all (for example a Linux
  // PC that doesn't let programs send pings). Without it every device would
  // just look down, with nothing to say why. The explanation and what to do
  // are the back end's words; they may hold a command to copy.
  import { icons } from "$lib/icons";
  import { status } from "$lib/status.svelte";
  import Icon from "./Icon.svelte";
</script>

{#if status.problem}
  <section class="alert problem" role="alert">
    <Icon path={icons.warn} size={18} />
    <div>
      <strong>Status checks aren't working, so up and down below can't be trusted.</strong>
      <p>{status.problem}</p>
    </div>
  </section>
{/if}

<style>
  .problem { display: flex; gap: 0.7rem; padding: 0.8rem 1rem; margin-bottom: var(--gap); border-width: 2px; }
  .problem :global(svg) { flex: none; margin-top: 0.1rem; color: var(--bad); }
  .problem div { min-width: 0; }
  strong { font-size: 1.05rem; }
  /* Kept as the back end wrote it, line breaks included, and selectable so a command can be copied. */
  p { margin: 0.3rem 0 0; white-space: pre-line; user-select: text; }
</style>

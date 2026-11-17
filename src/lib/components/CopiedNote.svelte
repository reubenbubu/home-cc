<script lang="ts">
  // The one "Copied" note for the whole app; see copied.svelte.ts. Also said to screen readers.
  import { copiedNote } from "$lib/copied.svelte";
</script>

<div class="live" role="status" aria-live="polite">
  {#if copiedNote.shown}{copiedNote.shown.ok ? "Copied" : "Couldn't copy"}{/if}
</div>
{#if copiedNote.shown}
  {#key copiedNote.shown.n}
    <div
      class="note"
      class:failed={!copiedNote.shown.ok}
      style:left="{copiedNote.shown.x}px"
      style:top="{copiedNote.shown.y}px"
      aria-hidden="true"
    >
      {copiedNote.shown.ok ? "Copied" : "Couldn't copy"}
    </div>
  {/key}
{/if}

<style>
  .live { position: absolute; width: 1px; height: 1px; overflow: hidden; clip-path: inset(50%); white-space: nowrap; }
  .note {
    position: fixed;
    transform: translate(-50%, calc(-100% - 0.5rem));
    padding: 0.15rem 0.5rem;
    border: 1px solid var(--accent);
    border-radius: var(--radius-sm);
    background: var(--panel-raised);
    color: var(--text);
    font-size: 0.85rem;
    white-space: nowrap;
    pointer-events: none;
    z-index: 100;
  }
  .note.failed { border-color: var(--bad); }
</style>

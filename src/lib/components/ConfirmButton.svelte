<script lang="ts">
  // A button for anything that removes or overrides something: the first click
  // only asks, the second one does it.
  let {
    label,
    confirmLabel,
    onconfirm,
    disabled = false,
    title,
    what,
  }: {
    label: string;
    confirmLabel: string;
    /** Should deal with its own errors; the button only waits for it. */
    onconfirm: () => unknown;
    /** Holds back both steps: nothing can be started, and a first click already given can't be completed. */
    disabled?: boolean;
    /** A tooltip on the first button, such as why it is held back. */
    title?: string;
    /** What the second click would do, for a button whose target can change while it waits (a list read again
     *  in the background). If it changes after the first click, the button goes back to asking, so the owner
     *  never confirms one thing and gets another. */
    what?: string;
  } = $props();

  let armed = $state(false);
  let busy = $state(false);
  /** `what` when the first click was given. */
  let armedFor = $state<string | undefined>(undefined);

  function arm() {
    armedFor = what;
    armed = true;
  }

  $effect(() => {
    if (armed && !busy && what !== armedFor) armed = false;
  });

  async function go() {
    // Changed in the moment before this click, ahead of the effect above: ask again rather than do it.
    if (what !== armedFor) {
      armed = false;
      return;
    }
    busy = true;
    try {
      await onconfirm();
    } finally {
      busy = false;
      armed = false;
    }
  }
</script>

{#if armed}
  <button type="button" class="btn small danger" onclick={go} disabled={busy || disabled}>{confirmLabel}</button>
  <button type="button" class="btn small" onclick={() => (armed = false)} disabled={busy}>Cancel</button>
{:else}
  <button type="button" class="btn small" onclick={arm} {disabled} {title}>{label}</button>
{/if}

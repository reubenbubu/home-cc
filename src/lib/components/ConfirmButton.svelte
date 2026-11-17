<script lang="ts">
  // A button for anything that removes or overrides something: the first click
  // only asks, the second one does it.
  let {
    label,
    confirmLabel,
    onconfirm,
    disabled = false,
    title,
  }: {
    label: string;
    confirmLabel: string;
    /** Should deal with its own errors; the button only waits for it. */
    onconfirm: () => unknown;
    /** Holds back both steps: nothing can be started, and a first click already given can't be completed. */
    disabled?: boolean;
    /** A tooltip on the first button, such as why it is held back. */
    title?: string;
  } = $props();

  let armed = $state(false);
  let busy = $state(false);

  async function go() {
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
  <button type="button" class="btn small" onclick={() => (armed = true)} {disabled} {title}>{label}</button>
{/if}

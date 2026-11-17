<script lang="ts">
  // Pausing blocking on one Pi-hole or on all of them: pick how long, then confirm, since ads and trackers get
  // through meanwhile. Which choice was open comes back after another screen was shown; the confirmation is
  // always asked again.
  import { untrack } from "svelte";
  import { drafts } from "$lib/drafts.svelte";
  import { PAUSES, pauseWords } from "$lib/pihole/logic";
  import { pihole } from "$lib/pihole/store.svelte";

  let {
    ids,
    target,
    label = "Pause blocking…",
    draftKey,
    onerror,
    open = $bindable(false),
    button = true,
    small = true,
  }: {
    /** The Pi-holes to pause. */
    ids: number[];
    /** Who is paused, in words: "Pi-hole 1", "both Pi-holes". */
    target: string;
    label?: string;
    draftKey: string;
    /** Called when a Pi-hole couldn't be paused; the store keeps why. */
    onerror?: () => void;
    /** The choices are shown. Bindable, so the button that opens them can sit elsewhere. */
    open?: boolean;
    /** Draws the button that opens the choices; false when the screen has its own. */
    button?: boolean;
    small?: boolean;
  } = $props();

  type Open = { secs: number | null; picked: boolean };
  const key = untrack(() => draftKey);
  const kept = drafts.get<Open>(key);
  if (kept) open = true;
  /** The pause chosen, waiting for the owner's yes. */
  let picked = $state<{ secs: number | null } | null>(kept?.picked ? { secs: kept.secs } : null);
  let busy = $state(false);
  $effect(() => drafts.remember<Open>(key, open ? { secs: picked?.secs ?? null, picked: !!picked } : null));

  const switching = $derived(ids.some((id) => pihole.switching.has(id)));

  function cancel() {
    open = false;
    picked = null;
  }

  async function go() {
    if (!picked) return;
    busy = true;
    try {
      const ok = await pihole.setBlocking(ids, false, picked.secs);
      if (!ok) onerror?.();
    } finally {
      busy = false;
      cancel();
      // Done: nothing to come back to. Said here as well, since the screen may have put this control away
      // (blocking is off now) before its own bookkeeping ran.
      drafts.clear(key);
    }
  }
</script>

{#if !open}
  {#if button}
    <button type="button" class="btn" class:small onclick={() => (open = true)} disabled={switching || ids.length === 0}>
      {switching ? "Pausing…" : label}
    </button>
  {/if}
{:else if !picked}
  <div class="choose" role="group" aria-label="How long to pause blocking on {target}">
    <span class="muted small">Pause blocking on {target}:</span>
    <div class="choices">
      {#each PAUSES as p (p.label)}
        <button type="button" class="btn small" onclick={() => (picked = { secs: p.secs })}>{p.label}</button>
      {/each}
      <button type="button" class="btn small quiet" onclick={cancel}>Cancel</button>
    </div>
  </div>
{:else}
  <div class="confirm alert warn" role="alert">
    <p>
      Pause blocking on <strong>{target}</strong> {pauseWords(picked.secs)}? Ads and trackers get through to every
      device that uses {ids.length > 1 ? "them" : "it"} until then.
    </p>
    <div class="choices">
      <button type="button" class="btn small danger" onclick={go} disabled={busy}>{busy ? "Pausing…" : "Yes, pause blocking"}</button>
      <button type="button" class="btn small" onclick={() => (picked = null)} disabled={busy}>Back</button>
      <button type="button" class="btn small quiet" onclick={cancel} disabled={busy}>Cancel</button>
    </div>
  </div>
{/if}

<style>
  .choose { display: flex; flex-direction: column; gap: 0.4rem; min-width: 0; }
  .choices { display: flex; gap: 0.4rem; flex-wrap: wrap; }
  .confirm { display: flex; flex-direction: column; gap: 0.55rem; margin: 0; }
  .confirm p { margin: 0; line-height: 1.5; }
  .quiet { border-color: transparent; color: var(--muted); }
</style>

<script lang="ts">
  // An IP address and, as a field of its own beside it, the hostname it resolves to. Every screen that shows
  // an address uses this, so the owner always sees which machine it is (see the Layout part of README.md).
  // Tables with room for a Hostname column draw the two halves in their own cells with `part`.
  // A double-click on either half copies it (Copyable); a state such as "looking…" is not copied.
  import { names } from "$lib/names.svelte";
  import Copyable from "./Copyable.svelte";

  let {
    host,
    known = null,
    part = "both",
    stack = false,
    class: className = "",
  }: {
    /** The IP address (or name) as stored. */
    host: string;
    /** A name the app already has for this device (its hostname from a scan); used before any lookup. */
    known?: string | null;
    /** "ip" or "name" draws only that half, for a table that gives the hostname its own column. */
    part?: "both" | "ip" | "name";
    /** The hostname under the address rather than beside it. */
    stack?: boolean;
    class?: string;
  } = $props();

  const found = $derived(part === "ip" ? null : names.get(host, known));

  const shown = $derived.by((): { text: string; tip: string; quiet: boolean; copy?: string } | null => {
    if (!found) return null;
    switch (found.kind) {
      case "name":
        return { text: found.name, tip: `Hostname: ${found.name}`, quiet: false, copy: found.name };
      case "looking":
        return { text: "looking…", tip: "Looking up the name of this address", quiet: true };
      case "none":
        return { text: "no name", tip: "Nothing on the network gives this address a name", quiet: true };
      case "failed":
        return { text: "name unknown", tip: `Couldn't look up the name of this address: ${found.error}`, quiet: true };
      case "notLookedUp":
        return { text: "name unknown", tip: "Names are only looked up for IPv4 addresses", quiet: true };
      case "notAddress":
        // Already a name: beside itself it would only repeat, but a Hostname column says it.
        return part === "name" ? { text: host, tip: host, quiet: false, copy: host } : null;
    }
  });
</script>

<span class="address {className}" class:stack>
  {#if part !== "name"}<Copyable class="ip mono" value={host} title="Address" />{/if}
  {#if shown?.copy}
    <!-- Said before the name to a screen reader, and kept outside what a double-click selects. -->
    {#if part !== "name"}<span class="sr-only">hostname </span>{/if}
    <Copyable class="hostname" value={shown.copy} title={shown.tip}>{shown.text}</Copyable>
  {:else if shown}
    <span class="hostname quiet" title={shown.tip}>{shown.text}</span>
  {/if}
</span>

<style>
  .address { display: inline-flex; align-items: baseline; gap: 0.55em; min-width: 0; max-width: 100%; vertical-align: bottom; }
  .address :global(.ip) { flex: none; white-space: nowrap; }
  /* The name gives way first in a tight space, with "…". */
  .address :global(.hostname) { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-family: var(--font); }
  .quiet { color: var(--muted); font-style: italic; }
  .stack { flex-direction: column; align-items: stretch; gap: 0; }
  .stack :global(.ip) { overflow: hidden; text-overflow: ellipsis; }
  .stack :global(.hostname) { font-size: 0.85em; line-height: 1.3; }
</style>

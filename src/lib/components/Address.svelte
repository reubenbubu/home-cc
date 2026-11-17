<script lang="ts">
  // An IP address and, as a field of its own beside it, the hostname it resolves to. Every screen that shows
  // an address uses this, so the owner always sees which machine it is (see the Layout part of README.md).
  // Tables with room for a Hostname column draw the two halves in their own cells with `part`.
  import { names } from "$lib/names.svelte";

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

  const shown = $derived.by((): { text: string; tip: string; quiet: boolean } | null => {
    if (!found) return null;
    switch (found.kind) {
      case "name":
        return { text: found.name, tip: `Hostname: ${found.name}`, quiet: false };
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
        return part === "name" ? { text: host, tip: host, quiet: false } : null;
    }
  });
</script>

<span class="address {className}" class:stack>
  {#if part !== "name"}<span class="ip mono">{host}</span>{/if}
  {#if shown}
    <span class="hostname" class:quiet={shown.quiet} title={shown.tip}>
      {#if part !== "name"}<span class="sr-only">hostname </span>{/if}{shown.text}
    </span>
  {/if}
</span>

<style>
  .address { display: inline-flex; align-items: baseline; gap: 0.55em; min-width: 0; max-width: 100%; vertical-align: bottom; }
  .ip { flex: none; white-space: nowrap; }
  /* The name gives way first in a tight space, with "…". */
  .hostname { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-family: var(--font); }
  .quiet { color: var(--muted); font-style: italic; }
  .stack { flex-direction: column; align-items: stretch; gap: 0; }
  .stack .ip { overflow: hidden; text-overflow: ellipsis; }
  .stack .hostname { font-size: 0.85em; line-height: 1.3; }
</style>

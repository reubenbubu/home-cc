<script lang="ts">
  // Whether the Pi-holes block from the same lists, and which lists are only on some of them. Nothing is changed
  // here: the owner adds or switches on a list in Pi-hole's own web interface (copying lists over may come later).
  import { icons } from "$lib/icons";
  import type { AdList } from "$lib/pihole/api";
  import { compareLists, differenceText, type Comparison, type Presence } from "$lib/pihole/logic";
  import Icon from "../Icon.svelte";

  let { hosts }: { hosts: { id: number; name: string; lists: AdList[] | null }[] } = $props();

  const result = $derived<Comparison | null>(compareLists(hosts));
  const unread = $derived(hosts.filter((h) => h.lists == null).map((h) => h.name));
  const MARK: Record<Presence, string> = { on: "on", off: "switched off", missing: "not there" };
  const both = $derived(result && result.compared.length === 2 ? "both" : "all of them");
</script>

<section class="panel compare" aria-label="Blocklists compared">
  <h2 class="section-title">Blocklists</h2>
  {#if !result}
    <p class="muted">
      {#if hosts.length < 2}
        With a second Pi-hole added, this shows whether both block from the same lists.
      {:else}
        The lists can be compared once {unread.join(" and ")} {unread.length === 1 ? "has" : "have"} been read.
      {/if}
    </p>
  {:else}
    {#if result.differences.length === 0}
      <p class="same"><Icon path={icons.check} size={15} /> The same {result.same === 1 ? "list is" : `${result.same} lists are`} switched on in {both}.</p>
    {:else}
      <p class="lead">
        <Icon path={icons.warn} size={15} />
        <span>
          {result.differences.length === 1 ? "1 list differs" : `${result.differences.length} lists differ`}
          between {result.compared.map((h) => h.name).join(" and ")}{result.same ? `; ${result.same} are the same on ${both}` : ""}.
          A device using the Pi-hole without a list gets through what that list would block.
        </span>
      </p>
      <ul class="rows">
        {#each result.differences as row (row.kind + row.address)}
          <li>
            <div class="addr mono">{row.address}</div>
            <div class="what small">
              <span class="kind">{row.kind === "block" ? "Blocklist" : "Allowlist"}</span>
              <span>{differenceText(row, result.compared)}</span>
            </div>
            <div class="marks small">
              {#each result.compared as h (h.id)}
                <span class="mark {row.on[h.id]}">{h.name}: {MARK[row.on[h.id]]}</span>
              {/each}
            </div>
          </li>
        {/each}
      </ul>
    {/if}
    <p class="muted small note">
      home-cc only compares; it doesn't change any lists. To add a list or switch it on, use the Pi-hole's web
      interface (Lists), then update its lists (Tools › Update Gravity). Copying lists from one Pi-hole to the other
      may come later.{unread.length ? ` Not compared: ${unread.join(", ")} (its lists couldn't be read).` : ""}
    </p>
  {/if}
</section>

<style>
  .compare { display: flex; flex-direction: column; gap: 0.7rem; min-width: 0; }
  .compare .section-title { margin: 0; }
  p { margin: 0; line-height: 1.5; }
  .same, .lead { display: flex; gap: 0.45rem; align-items: flex-start; color: var(--text-soft); }
  .same :global(svg) { color: var(--ok); flex: none; margin-top: 0.2rem; }
  .lead :global(svg) { color: var(--warn); flex: none; margin-top: 0.2rem; }
  .rows { list-style: none; margin: 0; padding: 0; display: grid; gap: 0.5rem; grid-template-columns: repeat(auto-fill, minmax(min(100%, 32rem), 1fr)); }
  .rows li { display: flex; flex-direction: column; gap: 0.3rem; padding: 0.6rem 0.75rem; border: 1px solid var(--border); border-radius: var(--radius-sm); min-width: 0; }
  .addr { overflow-wrap: anywhere; font-size: 0.9rem; user-select: text; }
  .what { display: flex; gap: 0.3rem 0.6rem; flex-wrap: wrap; color: var(--text-soft); }
  .kind { color: var(--muted); }
  .marks { display: flex; gap: 0.35rem; flex-wrap: wrap; }
  .mark { padding: 0.05rem 0.5rem; border-radius: 999px; border: 1px solid var(--border-strong); color: var(--text-soft); }
  .mark.on { border-color: color-mix(in srgb, var(--ok) 50%, transparent); }
  .mark.off { border-color: var(--warn); }
  .mark.missing { border-style: dashed; color: var(--muted); }
  .note { line-height: 1.5; }
</style>

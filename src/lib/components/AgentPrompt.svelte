<script lang="ts">
  // A small button under a problem: copies a prompt that asks a coding agent to find out what went wrong,
  // with what the app knows around it (see report.ts). The prompt can be shown, so the owner can read
  // what they are about to send, and copied by hand if the clipboard can't be used.
  import { icons } from "$lib/icons";
  import { buildPrompt, copyText } from "$lib/problems.svelte";
  import type { Details, Problem } from "$lib/report";
  import Icon from "./Icon.svelte";

  let { problem, extra }: { problem: Problem; extra?: () => Promise<Details> } = $props();

  let text = $state<string | null>(null);
  let state = $state<"idle" | "working" | "copied" | "failed">("idle");
  let open = $state(false);

  // A prompt made for an earlier message must not be shown, or copied, as if it were about this one.
  $effect(() => {
    void problem.what;
    void problem.error;
    text = null;
    state = "idle";
    open = false;
  });

  async function copy() {
    state = "working";
    const made = await buildPrompt(problem, extra);
    text = made;
    const copied = await copyText(made);
    // Only if the message wasn't replaced meanwhile.
    if (text !== made) return;
    state = copied ? "copied" : "failed";
    if (!copied) open = true;
  }
</script>

<div class="agent">
  <div class="row">
    <button
      class="ask"
      onclick={copy}
      disabled={state === "working"}
      title="Copies a prompt for a coding agent such as Claude Code, with this message and what home-cc knows around it"
    >
      <Icon path={icons.copy} size={14} />
      {state === "working" ? "Preparing…" : "Copy a prompt for an agent"}
    </button>
    {#if text}
      <button class="ask" aria-expanded={open} onclick={() => (open = !open)}>{open ? "Hide it" : "Show it"}</button>
    {/if}
    <span class="said" role="status">
      {#if state === "copied"}Copied. Paste it into Claude Code, ideally opened in your home-cc folder.
      {:else if state === "failed"}Couldn't copy it. Select the text below and copy it yourself.{/if}
    </span>
  </div>
  {#if text && open}
    <textarea
      class="mono"
      readonly
      rows="12"
      value={text}
      aria-label="Prompt for an agent"
      onfocus={(e) => e.currentTarget.select()}
    ></textarea>
  {/if}
</div>

<style>
  .agent { display: flex; flex-direction: column; gap: 0.4rem; margin-top: 0.45rem; }
  .row { display: flex; align-items: center; gap: 0.3rem 0.9rem; flex-wrap: wrap; }
  .ask {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    min-height: 1.9rem;
    padding: 0 0.1rem;
    border: none;
    background: none;
    color: var(--accent);
    font-size: 0.9rem;
    cursor: pointer;
  }
  .ask:hover { text-decoration: underline; }
  .ask:disabled { opacity: 0.6; cursor: wait; text-decoration: none; }
  .ask:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; border-radius: 3px; }
  .said { color: var(--muted); font-size: 0.88rem; }
  .said:empty { display: none; }
  textarea {
    width: 100%;
    min-height: 8rem;
    padding: 0.5rem 0.6rem;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-sm);
    background: var(--bg-sunk);
    font-size: 0.85rem;
    line-height: 1.45;
    resize: vertical;
    user-select: text;
  }
</style>

<script lang="ts">
  // Buttons under a problem that copy a prompt for a coding agent, with what the app knows around it (see
  // report.ts). A problem about a machine or service offers three: Investigate (look into it there and
  // report back), Remediate (fix it there) and I have a bug (find the fault in home-cc's code). A problem
  // about home-cc itself offers only the last, as one "Copy a prompt for an agent" button (promptKinds
  // decides). The prompt can be shown, so the owner can read what they are about to send, and copied by
  // hand if the clipboard can't be used.
  import { icons } from "$lib/icons";
  import { buildPrompt, copyText } from "$lib/problems.svelte";
  import { PROMPT_KINDS, promptKinds, type Details, type Problem, type PromptKind } from "$lib/report";
  import Icon from "./Icon.svelte";

  let { problem, extra }: { problem: Problem; extra?: () => Promise<Details> } = $props();

  const kinds = $derived(promptKinds(problem));

  let text = $state<string | null>(null);
  /** Which prompt is being made, or was made last. */
  let kind = $state<PromptKind>("bug");
  let phase = $state<"idle" | "working" | "copied" | "failed">("idle");
  let open = $state(false);

  // The screens hand in a new problem object whenever anything around it is redrawn (every few seconds
  // on a machine card), so only a change of the message itself counts: a string compares equal.
  const key = $derived(`${problem.area}\n${problem.what}\n${problem.error}`);
  /** Moves on when the message changes, so a prompt still being made for the old one is dropped. */
  let generation = 0;

  // A prompt made for an earlier message must not be shown, or copied, as if it were about this one.
  $effect(() => {
    void key;
    generation++;
    text = null;
    phase = "idle";
    open = false;
  });

  async function copy(k: PromptKind) {
    const gen = ++generation;
    kind = k;
    phase = "working";
    const made = await buildPrompt(problem, extra, k);
    if (gen !== generation) return;
    text = made;
    const copied = await copyText(made);
    if (gen !== generation) return;
    phase = copied ? "copied" : "failed";
    if (!copied) open = true;
  }

  const one = $derived(kinds.length === 1);
</script>

<div class="agent">
  <div class="row">
    {#if one}
      <button class="ask" onclick={() => copy("bug")} disabled={phase === "working"} title={PROMPT_KINDS.bug.asks}>
        <Icon path={icons.copy} size={14} />
        {phase === "working" ? "Preparing…" : "Copy a prompt for an agent"}
      </button>
    {:else}
      <div class="kinds" role="group" aria-label="Copy a prompt for an agent">
        <span class="lead" aria-hidden="true"><Icon path={icons.copy} size={14} /> Copy a prompt for an agent:</span>
        {#each kinds as k (k)}
          <button class="ask" onclick={() => copy(k)} disabled={phase === "working"} title={PROMPT_KINDS[k].asks}>
            {phase === "working" && kind === k ? "Preparing…" : PROMPT_KINDS[k].label}
          </button>
        {/each}
      </div>
    {/if}
    {#if text}
      <button class="ask" aria-expanded={open} onclick={() => (open = !open)}>{open ? "Hide it" : "Show it"}</button>
    {/if}
    <span class="said" role="status">
      {#if phase === "copied"}
        {#if kind === "bug"}Copied{one ? "" : ` "${PROMPT_KINDS.bug.label}"`}. Paste it into Claude Code, ideally opened in your home-cc folder.
        {:else}Copied "{PROMPT_KINDS[kind].label}". Paste it into Claude Code on this PC, which can reach the machine.{/if}
      {:else if phase === "failed"}Couldn't copy it. Select the text below and copy it yourself.{/if}
    </span>
  </div>
  {#if text && open}
    <textarea
      class="mono"
      readonly
      rows="12"
      value={text}
      aria-label={one ? "Prompt for an agent" : `Prompt for an agent: ${PROMPT_KINDS[kind].label}`}
      onfocus={(e) => e.currentTarget.select()}
    ></textarea>
  {/if}
</div>

<style>
  .agent { display: flex; flex-direction: column; gap: 0.4rem; margin-top: 0.45rem; }
  .row { display: flex; align-items: center; gap: 0.3rem 0.9rem; flex-wrap: wrap; }
  .kinds { display: flex; align-items: center; gap: 0.1rem 0.8rem; flex-wrap: wrap; min-width: 0; }
  .lead { display: inline-flex; align-items: center; gap: 0.35rem; color: var(--muted); font-size: 0.9rem; }
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

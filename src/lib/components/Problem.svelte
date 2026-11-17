<script lang="ts">
  // A problem on a screen: the message, then a way to hand it to a coding agent (AgentPrompt).
  // Shown problems are remembered for the session, so a later prompt can mention what came before.
  import type { Snippet } from "svelte";
  import { problems } from "$lib/problems.svelte";
  import type { Details, Problem } from "$lib/report";
  import AgentPrompt from "./AgentPrompt.svelte";

  let {
    problem,
    message,
    extra,
    tone,
    class: cls = "",
    children,
  }: {
    problem: Problem;
    /** Shown instead of problem.error, such as 'Not saved: …'. */
    message?: string;
    /** More details, read only when a prompt is made (for example whether a password is stored). */
    extra?: () => Promise<Details>;
    tone?: "warn";
    class?: string;
    /** Shown instead of the message, for a problem that needs more than a line. */
    children?: Snippet;
  } = $props();

  $effect(() => problems.record({ area: problem.area, what: problem.what, error: problem.error }));
</script>

<div class="alert problem {cls}" class:warn={tone === "warn"} role="alert">
  {#if children}
    {@render children()}
  {:else}
    <p class="message">{message ?? problem.error}</p>
  {/if}
  <AgentPrompt {problem} {extra} />
</div>

<style>
  .problem :global(p) { margin: 0; }
  .message { white-space: pre-line; }
</style>

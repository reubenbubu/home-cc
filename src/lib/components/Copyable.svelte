<script lang="ts">
  // A piece of text that copies itself to the clipboard on double-click, such as an IP address or a
  // hostname. Used for every address and name on every screen (the Address component wraps both), so it
  // behaves the same everywhere. The whole text is selected too, so it is clear what was copied: a plain
  // double-click on "192.168.68.54" would select only part of it.
  import type { Snippet } from "svelte";
  import { copyText } from "$lib/clipboard";

  let {
    value,
    class: cls = "",
    title,
    children,
  }: {
    /** What is copied. */
    value: string;
    class?: string;
    /** Hover text; "Double-click to copy" is added to it. */
    title?: string;
    /** What is shown; the value itself when left out. */
    children?: Snippet;
  } = $props();

  let said = $state<"copied" | "failed" | null>(null);
  let timer: ReturnType<typeof setTimeout> | undefined;

  async function copy(e: MouseEvent) {
    const node = e.currentTarget as HTMLElement;
    const range = document.createRange();
    range.selectNodeContents(node.querySelector(".text") ?? node);
    const selection = getSelection();
    selection?.removeAllRanges();
    selection?.addRange(range);
    said = (await copyText(value)) ? "copied" : "failed";
    clearTimeout(timer);
    timer = setTimeout(() => (said = null), 1400);
  }

  $effect(() => () => clearTimeout(timer));
</script>

<!-- Not a button and not in the tab order: it is a convenience on top of ordinary selectable text, which
     the keyboard can still select and copy with Ctrl+C. -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<span
  class="copyable {cls}"
  title={title ? `${title}\nDouble-click to copy` : "Double-click to copy"}
  ondblclick={copy}
>
  <span class="text">{#if children}{@render children()}{:else}{value}{/if}</span>
  {#if said}
    <span class="said" class:failed={said === "failed"} role="status">{said === "copied" ? "Copied" : "Couldn't copy"}</span>
  {/if}
</span>

<style>
  .copyable { position: relative; cursor: copy; user-select: text; min-width: 0; }
  .said {
    position: absolute;
    left: 50%;
    bottom: calc(100% + 0.25rem);
    transform: translateX(-50%);
    padding: 0.1rem 0.45rem;
    border-radius: var(--radius-sm);
    background: var(--panel-raised);
    border: 1px solid var(--accent);
    color: var(--text);
    font-family: var(--font);
    font-size: 0.8rem;
    white-space: nowrap;
    pointer-events: none;
    z-index: 5;
  }
  .said.failed { border-color: var(--bad); }
</style>

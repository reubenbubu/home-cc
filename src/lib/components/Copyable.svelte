<script lang="ts">
  // A piece of text that copies itself to the clipboard on double-click, such as an IP address or a
  // hostname. Every address and name on every screen goes through this (Address uses it for both halves),
  // so it behaves the same everywhere. The whole text is selected too, so it is clear what was copied: a
  // plain double-click on "192.168.68.54" would select only part of it. The "Copied" note is drawn by
  // CopiedNote, outside any cell that could cut it off.
  import type { Snippet } from "svelte";
  import { copyText } from "$lib/clipboard";
  import { copiedNote } from "$lib/copied.svelte";

  let {
    value,
    class: cls = "",
    title,
    children,
  }: {
    /** What is copied. */
    value: string;
    /** Classes for the text itself, such as one that cuts it short with "…". */
    class?: string;
    /** Hover text; "Double-click to copy" is added to it. */
    title?: string;
    /** What is shown; the value itself when left out. */
    children?: Snippet;
  } = $props();

  async function copy(e: MouseEvent) {
    const node = e.currentTarget as HTMLElement;
    const range = document.createRange();
    range.selectNodeContents(node);
    const selection = getSelection();
    selection?.removeAllRanges();
    selection?.addRange(range);
    const { clientX: x, clientY: y } = e;
    copiedNote.show(x, y, await copyText(value));
  }
</script>

<!-- Not a button and not in the tab order: it is a convenience on top of ordinary selectable text, which
     the keyboard can still select and copy with Ctrl+C. -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<span
  class="copyable {cls}"
  title={title ? `${title}\nDouble-click to copy` : "Double-click to copy"}
  ondblclick={copy}
>{#if children}{@render children()}{:else}{value}{/if}</span>

<style>
  .copyable { cursor: copy; user-select: text; }
</style>

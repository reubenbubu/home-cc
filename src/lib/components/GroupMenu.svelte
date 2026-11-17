<script lang="ts">
  // The quick way to put a machine in a group, from its card on the Machines screen: a small menu of the
  // groups there are, "No group", and "New group…" (which opens the machine's form at its Group field, so
  // a new name is typed where every other detail of the machine is). Works with the keyboard as a menu
  // does: Enter, Space or the down arrow opens it, the arrows move, Enter picks, Escape closes.
  import { tick } from "svelte";
  import type { Device } from "$lib/api";
  import { icons } from "$lib/icons";
  import Icon from "./Icon.svelte";

  let {
    device,
    groups,
    onpick,
    onnew,
  }: {
    device: Device;
    /** Every group on the screen, in heading order. */
    groups: string[];
    /** A group name, or "" for none. Should deal with its own errors. */
    onpick: (group: string) => unknown;
    onnew: () => unknown;
  } = $props();

  let open = $state(false);
  let root = $state<HTMLElement>();
  let menu = $state<HTMLElement>();
  let button = $state<HTMLButtonElement>();
  const current = $derived(device.group ?? "");

  const items = () => [...(menu?.querySelectorAll<HTMLElement>("[role^='menuitem']") ?? [])];

  async function show(at: "current" | "first" | "last" = "current") {
    open = true;
    await tick();
    const list = items();
    const checked = list.find((el) => el.getAttribute("aria-checked") === "true");
    (at === "last" ? list.at(-1) : at === "first" ? list[0] : (checked ?? list[0]))?.focus();
  }

  function close(refocus: boolean) {
    if (!open) return;
    open = false;
    if (refocus) button?.focus();
  }

  function onButtonKey(e: KeyboardEvent) {
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      show(e.key === "ArrowUp" ? "last" : "current");
    }
  }

  function onMenuKey(e: KeyboardEvent) {
    const list = items();
    const at = list.indexOf(document.activeElement as HTMLElement);
    let next: number | null = null;
    switch (e.key) {
      case "ArrowDown":
        next = (at + 1) % list.length;
        break;
      case "ArrowUp":
        next = (at - 1 + list.length) % list.length;
        break;
      case "Home":
        next = 0;
        break;
      case "End":
        next = list.length - 1;
        break;
      case "Escape":
        e.preventDefault();
        // Not also the end of a card being moved, or anything else listening for Escape.
        e.stopPropagation();
        close(true);
        return;
      case "Tab":
        close(false);
        return;
    }
    if (next !== null) {
      e.preventDefault();
      list[next]?.focus();
    }
  }

  // A press anywhere else closes it.
  $effect(() => {
    if (!open) return;
    const away = (e: PointerEvent) => {
      if (!root?.contains(e.target as Node)) close(false);
    };
    document.addEventListener("pointerdown", away, true);
    return () => document.removeEventListener("pointerdown", away, true);
  });

  function pick(group: string) {
    close(true);
    if (group !== current) onpick(group);
  }

  function startNew() {
    close(false);
    onnew();
  }
</script>

<span class="group-menu" bind:this={root}>
  <button
    bind:this={button}
    type="button"
    class="icon-btn"
    data-group-menu={device.id}
    aria-haspopup="menu"
    aria-expanded={open}
    aria-label="Group for {device.name}"
    title={current ? `In the group ${current}` : "Put in a group"}
    onclick={() => (open ? close(true) : show())}
    onkeydown={onButtonKey}
  >
    <Icon path={icons.folder} size={16} />
  </button>
  {#if open}
    <div class="menu" role="menu" aria-label="Group for {device.name}" tabindex="-1" bind:this={menu} onkeydown={onMenuKey}>
      {#each groups as group (group)}
        <button type="button" role="menuitemradio" aria-checked={group === current} tabindex="-1" onclick={() => pick(group)}>
          <span class="mark">{#if group === current}<Icon path={icons.check} size={14} />{/if}</span>
          <span class="ellipsis" title={group}>{group}</span>
        </button>
      {/each}
      <button type="button" role="menuitemradio" aria-checked={current === ""} tabindex="-1" onclick={() => pick("")}>
        <span class="mark">{#if current === ""}<Icon path={icons.check} size={14} />{/if}</span>
        <span>No group</span>
      </button>
      <div class="line" role="separator"></div>
      <button type="button" role="menuitem" tabindex="-1" onclick={startNew}>
        <span class="mark"></span>
        <span>New group…</span>
      </button>
    </div>
  {/if}
</span>

<style>
  .group-menu { position: relative; display: inline-flex; }
  .menu {
    position: absolute;
    top: calc(100% + 0.25rem);
    right: 0;
    z-index: 30;
    display: flex;
    flex-direction: column;
    width: max-content;
    min-width: 11rem;
    max-width: min(18rem, 70vw);
    padding: 0.3rem;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-sm);
    background: var(--panel-raised);
    box-shadow: 0 10px 28px rgb(0 0 0 / 0.45);
  }
  .menu button {
    display: flex;
    align-items: center;
    gap: 0.45rem;
    min-width: 0;
    min-height: 2.25rem;
    padding: 0 0.7rem 0 0.4rem;
    border: 0;
    border-radius: var(--radius-sm);
    background: none;
    color: var(--text);
    text-align: left;
    cursor: pointer;
  }
  .menu button:hover, .menu button:focus-visible { background: var(--panel); outline: none; }
  .menu button:focus-visible { box-shadow: inset 0 0 0 2px var(--accent); }
  .mark { display: inline-grid; place-items: center; width: 1rem; flex: none; color: var(--accent); }
  .line { height: 1px; margin: 0.25rem 0.3rem; background: var(--border-strong); }
</style>

<script lang="ts">
  import "../app.css";
  import { page } from "$app/state";
  import { onMount } from "svelte";
  import { modules } from "$lib/modules";
  import { devices } from "$lib/devices.svelte";
  import { live, startLive } from "$lib/live.svelte";
  import { settings } from "$lib/settings.svelte";
  import { stats } from "$lib/stats.svelte";
  import { status } from "$lib/status.svelte";
  import Icon from "$lib/components/Icon.svelte";

  let { children } = $props();

  // One load of everything, then the stores follow the back end's events.
  onMount(() => startLive());

  /** True in a browser with the sample back end instead of the real one. */
  const sample = "__HOME_CC_MOCK__" in window;

  /** Loading problems, shown on every screen so stale numbers are never taken for live ones. */
  const problems = $derived(
    [
      devices.error && `Couldn't load devices: ${devices.error}`,
      status.error && `Couldn't load device status: ${status.error}`,
      stats.error && `Couldn't load machine stats: ${stats.error}`,
      settings.error && `Couldn't load settings: ${settings.error}`,
      live.error && `Live updates aren't working, so this screen may be out of date: ${live.error}`,
    ].filter((p): p is string => !!p),
  );

  function isActive(href: string) {
    return href === "/" ? page.url.pathname === "/" : page.url.pathname.startsWith(href);
  }
</script>

<div class="shell">
  <nav class="sidebar" aria-label="Main">
    <div class="brand">
      <div class="logo" aria-hidden="true"></div>
      <span class="brand-name">home-cc</span>
    </div>
    {#each modules as m}
      <a
        href={m.href}
        class="nav-item"
        class:footer={m.footer}
        class:active={isActive(m.href)}
        aria-current={isActive(m.href) ? "page" : undefined}
        title={m.label}
      >
        <Icon path={m.icon} />
        <span class="nav-label">{m.label}</span>
      </a>
    {/each}
    {#if sample}<div class="sample" title="Not connected to the app: this is made-up sample data">Sample<span class="nav-label">&nbsp;data</span></div>{/if}
  </nav>
  <main class="content">
    {#each problems as problem}
      <p class="alert load-problem" role="alert">{problem}</p>
    {/each}
    {@render children()}
  </main>
</div>

<style>
  .shell { display: flex; height: 100vh; }

  .sidebar {
    width: 220px;
    flex: none;
    padding: 1.5rem 0.9rem;
    background: var(--bg-sunk);
    border-right: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
    overflow-y: auto;
  }
  .brand { display: flex; align-items: center; gap: 0.75rem; padding: 0 0.6rem 1.5rem; }
  .logo { width: 26px; height: 26px; border-radius: 6px; background: var(--accent); flex: none; }
  .brand-name { font-weight: 700; font-size: 1.15rem; letter-spacing: 0.02em; }

  .nav-item {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    min-height: 2.75rem;
    flex: none;
    padding: 0 0.75rem;
    border-radius: var(--radius-sm);
    color: #9ba6b2;
    text-decoration: none;
    font-weight: 500;
  }
  .nav-item:hover { background: var(--panel); color: var(--text); }
  .nav-item.active { background: var(--panel-raised); color: var(--text); }
  .nav-item.active :global(svg) { color: var(--accent); }
  .nav-item.footer { margin-top: auto; }

  .sample {
    margin-top: 0.5rem;
    padding: 0.3rem 0.4rem;
    border: 1px dashed var(--warn);
    border-radius: var(--radius-sm);
    color: var(--warn);
    font-size: 0.85rem;
    text-align: center;
    white-space: nowrap;
    overflow: hidden;
  }

  .content {
    flex: 1;
    min-width: 0;
    overflow-y: auto;
    padding: 1.5rem 1.85rem;
  }
  .load-problem { margin-bottom: var(--gap); }

  /* Narrow windows (for example half of a 1080p screen): icons only. */
  @media (max-width: 1180px) {
    .sidebar { width: 64px; padding-inline: 0.6rem; }
    .brand { justify-content: center; padding-inline: 0; }
    .brand-name, .nav-label { display: none; }
    .nav-item { justify-content: center; padding: 0; }
  }
</style>

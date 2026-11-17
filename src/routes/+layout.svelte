<script lang="ts">
  import "../app.css";
  import { page } from "$app/state";
  import { onMount } from "svelte";
  import { errorText } from "$lib/api";
  import { modules } from "$lib/modules";
  import { devices } from "$lib/devices.svelte";
  import { live, startLive } from "$lib/live.svelte";
  import { settings } from "$lib/settings.svelte";
  import { stats } from "$lib/stats.svelte";
  import { status } from "$lib/status.svelte";
  import { zoom } from "$lib/zoom.svelte";
  import Icon from "$lib/components/Icon.svelte";
  import CopiedNote from "$lib/components/CopiedNote.svelte";
  import Problem from "$lib/components/Problem.svelte";
  import type { Problem as ProblemInfo } from "$lib/report";

  let { children } = $props();

  // One load of everything, then the stores follow the back end's events.
  onMount(() => startLive());

  // The size the owner left the app at, before the first screen is seen (the app stays out of sight until
  // the window has zoomed); then the zoom keys work everywhere.
  zoom.restore();
  onMount(() => zoom.listen());

  /** True in a browser with the sample back end instead of the real one. */
  const sample = "__HOME_CC_MOCK__" in window;

  /** Loading problems, shown on every screen so stale numbers are never taken for live ones. */
  const problems = $derived(
    [
      devices.error && { area: "devices", what: "Loading the saved devices", lead: "Couldn't load devices", error: devices.error },
      status.error && { area: "status", what: "Loading device status", lead: "Couldn't load device status", error: status.error },
      stats.error && { area: "stats", what: "Loading machine stats", lead: "Couldn't load machine stats", error: stats.error },
      settings.error && { area: "settings", what: "Loading the settings and this PC's networks", lead: "Couldn't load settings", error: settings.error },
      live.error && {
        area: "live",
        what: "Listening for updates from the back end",
        lead: "Live updates aren't working, so this screen may be out of date",
        error: live.error,
      },
    ].filter((p): p is ProblemInfo & { lead: string } => !!p),
  );

  /** What a screen that couldn't be drawn hands to an agent. The stack says where in the screens it broke. */
  function screenProblem(error: unknown): ProblemInfo {
    return {
      area: "screen",
      what: `Showing the ${page.url.pathname} screen, which failed while drawing`,
      error: errorText(error),
      trace: error instanceof Error ? error.stack : undefined,
    };
  }

  /** A screen that fails while drawing is replaced by a message; the cause goes to the log for a bug report. */
  function screenFailed(error: unknown) {
    console.error("A screen could not be shown:", error);
  }

  function isActive(href: string) {
    return href === "/" ? page.url.pathname === "/" : page.url.pathname.startsWith(href);
  }
</script>

<div class="shell" class:zooming={!zoom.ready}>
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
    <!-- A size that couldn't be set by a key or the wheel, or put back as the app started. Settings shows
         it next to its own size buttons instead. -->
    {#if zoom.problem && !page.url.pathname.startsWith("/settings")}
      <div class="zoom-problem">
        <Problem
          problem={{ area: "zoom", what: zoom.problem.what, error: zoom.problem.error }}
          message="{zoom.problem.what}: {zoom.problem.error}"
        />
        <button class="btn" type="button" onclick={() => zoom.dismiss()}>OK</button>
      </div>
    {/if}
    {#each problems as problem (problem.area)}
      <Problem class="load-problem" {problem} message="{problem.lead}: {problem.error}" />
    {/each}
    <!-- Without this, one value a screen can't draw would leave it blank or stuck, with nothing said.
         A new boundary for each screen, so moving to another screen always works. -->
    {#key page.route.id}
      <svelte:boundary onerror={screenFailed}>
        {@render children()}
        {#snippet failed(error, reset)}
          <section class="panel broken" role="alert">
            <h1>This screen couldn't be shown</h1>
            <p>Something home-cc was given isn't in a form this screen can show. The other screens still work.</p>
            <Problem problem={screenProblem(error)} message="What went wrong: {errorText(error)}" />
            <div><button class="btn" onclick={reset}>Try again</button></div>
          </section>
        {/snippet}
      </svelte:boundary>
    {/key}
  </main>
</div>
<CopiedNote />

<style>
  /* --page-zoom is only set in a browser with the sample data, where zoom.svelte.ts scales the page, which
     also scales vh. In the app the window itself zooms and this is plain 100vh. */
  .shell { display: flex; height: calc(100vh / var(--page-zoom, 1)); }
  .shell.zooming { visibility: hidden; }

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
  .content :global(.load-problem) { margin-bottom: var(--gap); }
  /* Stays in view while the screen scrolls, as it can come from a key pressed anywhere on it. */
  .zoom-problem {
    position: sticky;
    top: 0;
    z-index: 2;
    display: flex;
    align-items: flex-start;
    gap: 0.75rem;
    margin-bottom: var(--gap);
    background: var(--bg);
  }
  .zoom-problem :global(.alert) { flex: 1; min-width: 0; }
  .zoom-problem .btn { flex: none; }
  .broken { display: flex; flex-direction: column; gap: 0.75rem; max-width: 46rem; border-color: var(--bad); }
  .broken h1 { margin: 0; font-size: 1.3rem; font-weight: 600; }
  .broken p { margin: 0; line-height: 1.5; overflow-wrap: anywhere; }

  /* Narrow windows (for example half of a 1080p screen): icons only. */
  @media (max-width: 1180px) {
    .sidebar { width: 64px; padding-inline: 0.6rem; }
    .brand { justify-content: center; padding-inline: 0; }
    .brand-name, .nav-label { display: none; }
    .nav-item { justify-content: center; padding: 0; }
  }
  /* A small window zoomed right in: less room lost around the edges. */
  @media (max-width: 600px) {
    .content { padding: 1rem; }
  }
</style>

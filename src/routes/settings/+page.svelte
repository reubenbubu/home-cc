<script lang="ts">
  import { onMount } from "svelte";
  import { api, errorText, secretKeys, type AppInfo, type Settings } from "$lib/api";
  import { icons } from "$lib/icons";
  import { settings } from "$lib/settings.svelte";
  import { zoom } from "$lib/zoom.svelte";
  import { ZOOM_DEFAULT, ZOOM_MAX, ZOOM_MIN } from "$lib/zoom";
  import ConfirmButton from "$lib/components/ConfirmButton.svelte";
  import Icon from "$lib/components/Icon.svelte";

  type RangeChoice = "auto" | "custom" | string;

  let form = $state<Settings>({ ...settings.value });
  let rangeChoice = $state<RangeChoice>("auto");
  let customRange = $state("");
  let filled = false;
  let saving = $state(false);
  let saveError = $state<string | null>(null);
  let savedNote = $state(false);
  /** Field messages only show once Save has been tried. */
  let tried = $state(false);

  let passphrase = $state("");
  let hasPassphrase = $state<boolean | null>(null);
  let secretError = $state<string | null>(null);
  let storing = $state(false);
  let info = $state<AppInfo | null>(null);
  let infoError = $state<string | null>(null);

  function fill(s: Settings) {
    form = { ...s };
    if (!s.scanRange) rangeChoice = "auto";
    else if (settings.ranges.some((r) => r.cidr === s.scanRange)) rangeChoice = s.scanRange;
    else {
      rangeChoice = "custom";
      customRange = s.scanRange;
    }
  }

  // The saved values may arrive after this screen opens: fill the form once they are in.
  $effect(() => {
    if (settings.loaded && !filled) {
      filled = true;
      fill(settings.value);
    }
  });

  // A range picked from the list whose network has since gone (the list is read again below and after
  // every scan) is kept as typed text, so the choice never shows as an empty box.
  $effect(() => {
    if (rangeChoice !== "auto" && rangeChoice !== "custom" && !settings.ranges.some((r) => r.cidr === rangeChoice)) {
      customRange = rangeChoice;
      rangeChoice = "custom";
    }
  });

  onMount(() => {
    // This PC's networks may have changed since the app started.
    settings.loadSubnets();
    api.hasSecret(secretKeys.sshKeyPassphrase).then(
      (stored) => (hasPassphrase = stored),
      (e) => (secretError = `Couldn't check for a stored passphrase: ${errorText(e)}`),
    );
    api.appInfo().then(
      (i) => (info = i),
      (e) => (infoError = errorText(e)),
    );
  });

  const RANGE = /^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})\/(\d{1,2})$/;

  function rangeProblem(text: string): string | null {
    const m = RANGE.exec(text.trim());
    if (!m) return 'Write the range like 192.168.68.0/22: an address, a slash, then a number.';
    if (m.slice(1, 5).some((n) => Number(n) > 255)) return "Each of the four numbers in the address must be 255 or less.";
    if (Number(m[5]) > 32) return "The number after the slash must be 32 or less.";
    return null;
  }

  /** Not an error: the back end decides what it will scan. A smaller number after the slash means more addresses. */
  const bigRange = $derived(rangeChoice === "custom" && !rangeProblem(customRange) && Number(customRange.trim().split("/")[1]) < 20);

  const whole = (n: unknown, min: number, max: number) => typeof n === "number" && Number.isInteger(n) && n >= min && n <= max;

  const problems = $derived({
    ping: whole(form.pingIntervalSecs, 5, 3600) ? null : "Use a whole number of seconds from 5 to 3600 (1 hour).",
    stats: whole(form.statsIntervalSecs, 10, 3600) ? null : "Use a whole number of seconds from 10 to 3600 (1 hour).",
    disk: whole(form.diskWarnPct, 1, 100) ? null : "Use a whole number from 1 to 100.",
    ram: whole(form.ramWarnPct, 1, 100) ? null : "Use a whole number from 1 to 100.",
    range: rangeChoice === "custom" ? rangeProblem(customRange) : null,
  });
  const valid = $derived(Object.values(problems).every((p) => p === null));

  async function save(e: SubmitEvent) {
    e.preventDefault();
    tried = true;
    savedNote = false;
    saveError = null;
    if (!valid) return;
    saving = true;
    try {
      const scanRange = rangeChoice === "auto" ? "" : rangeChoice === "custom" ? customRange.trim() : rangeChoice;
      fill(await settings.save({ ...form, scanRange }));
      savedNote = true;
      tried = false;
    } catch (err) {
      saveError = errorText(err);
    } finally {
      saving = false;
    }
  }

  async function storePassphrase() {
    if (!passphrase) return;
    storing = true;
    secretError = null;
    try {
      await api.setSecret(secretKeys.sshKeyPassphrase, passphrase);
      hasPassphrase = true;
      passphrase = "";
    } catch (err) {
      secretError = `Couldn't store the passphrase: ${errorText(err)}`;
    } finally {
      storing = false;
    }
  }

  async function removePassphrase() {
    secretError = null;
    try {
      await api.deleteSecret(secretKeys.sshKeyPassphrase);
      hasPassphrase = false;
    } catch (err) {
      secretError = `Couldn't remove the passphrase: ${errorText(err)}`;
    }
  }
</script>

{#snippet problem(text: string | null, id: string)}
  {#if tried && text}<span class="problem" {id}>{text}</span>{/if}
{/snippet}

<header class="page-head">
  <h1>Settings</h1>
</header>

<form onsubmit={save} oninput={() => (savedNote = false)} novalidate>
  <div class="grid">
    <section class="panel">
      <h2 class="section-title">Checking</h2>
      <label>How often to check that every device is there
        <span class="with-unit">
          <input class="mono" type="number" min="5" max="3600" step="1" bind:value={form.pingIntervalSecs}
            aria-invalid={tried && !!problems.ping} aria-describedby="p-ping" />
          <span class="muted">seconds</span>
        </span>
      </label>
      {@render problem(problems.ping, "p-ping")}
      <label>How often to read CPU, RAM and disk from each machine
        <span class="with-unit">
          <input class="mono" type="number" min="10" max="3600" step="1" bind:value={form.statsIntervalSecs}
            aria-invalid={tried && !!problems.stats} aria-describedby="p-stats" />
          <span class="muted">seconds</span>
        </span>
      </label>
      {@render problem(problems.stats, "p-stats")}
      <p class="hint">Shorter times notice a problem sooner but put a little more traffic on the network.</p>
    </section>

    <section class="panel">
      <h2 class="section-title">Network scan</h2>
      <label>Range to scan
        <select bind:value={rangeChoice}>
          <option value="auto">Automatic{settings.bestSubnet ? ` (now ${settings.bestSubnet.cidr})` : ""}</option>
          {#each settings.ranges as r}
            <option value={r.cidr}>{r.label}</option>
          {/each}
          <option value="custom">Something else…</option>
        </select>
      </label>
      {#if rangeChoice === "custom"}
        <label>Range
          <input class="mono" bind:value={customRange} placeholder="192.168.68.0/22" spellcheck="false" autocomplete="off"
            aria-invalid={tried && !!problems.range} aria-describedby="p-range" />
        </label>
        {@render problem(problems.range, "p-range")}
        {#if bigRange}<p class="hint">That is a big range: a scan of it may take a long time, or be refused.</p>{/if}
      {/if}
      <p class="hint">
        Automatic picks the network this PC is on. Choose one yourself if the app picks the wrong one, for example a
        virtual adapter.
      </p>
    </section>

    <section class="panel">
      <h2 class="section-title">Warnings</h2>
      <label>Warn when a disk is fuller than
        <span class="with-unit">
          <input class="mono" type="number" min="1" max="100" step="1" bind:value={form.diskWarnPct}
            aria-invalid={tried && !!problems.disk} aria-describedby="p-disk" />
          <span class="muted">percent</span>
        </span>
      </label>
      {@render problem(problems.disk, "p-disk")}
      <label>Warn when RAM use is above
        <span class="with-unit">
          <input class="mono" type="number" min="1" max="100" step="1" bind:value={form.ramWarnPct}
            aria-invalid={tried && !!problems.ram} aria-describedby="p-ram" />
          <span class="muted">percent</span>
        </span>
      </label>
      {@render problem(problems.ram, "p-ram")}
    </section>

    <section class="panel">
      <h2 class="section-title">SSH login</h2>
      <label>Key file
        <input class="mono" bind:value={form.sshKeyPath} placeholder="Leave empty to use the usual ones in your .ssh folder"
          spellcheck="false" autocomplete="off" />
      </label>
      <p class="hint">
        The private key home-cc uses to log in to your machines, for example
        <span class="mono">C:\Users\you\.ssh\id_ed25519</span>.
      </p>

      <div class="field">
        <label for="passphrase">Passphrase for the key file</label>
        <div class="with-unit">
          <input id="passphrase" type="password" bind:value={passphrase} autocomplete="new-password"
            placeholder={hasPassphrase ? "Type a new one to replace it" : "Only if your key has one"}
            onkeydown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                storePassphrase();
              }
            }} />
          <button class="btn" type="button" onclick={storePassphrase} disabled={!passphrase || storing}>
            {storing ? "Storing…" : hasPassphrase ? "Replace" : "Store"}
          </button>
        </div>
        {#if hasPassphrase}
          <div class="stored">
            <span><Icon path={icons.check} size={14} /> A passphrase is stored.</span>
            <span class="stored-actions">
              <ConfirmButton label="Remove" confirmLabel="Remove passphrase" onconfirm={removePassphrase} />
            </span>
          </div>
        {/if}
        {#if secretError}<p class="alert" role="alert">{secretError}</p>{/if}
        <p class="hint">Kept in your system's keychain, not in the settings file, and never shown again.</p>
      </div>
    </section>

    <!-- Not saved with the form: a change here applies straight away. -->
    <section class="panel">
      <h2 class="section-title">Display</h2>
      <div class="field">
        <span id="zoom-label">Size of everything in the window</span>
        <div class="zoom" role="group" aria-labelledby="zoom-label">
          <button class="btn step" type="button" onclick={() => zoom.act("out")} disabled={zoom.percent <= ZOOM_MIN}
            aria-label="Smaller" title="Smaller (Ctrl and -)">−</button>
          <output class="mono zoom-now" aria-live="polite">{zoom.percent}%</output>
          <button class="btn step" type="button" onclick={() => zoom.act("in")} disabled={zoom.percent >= ZOOM_MAX}
            aria-label="Bigger" title="Bigger (Ctrl and +)">+</button>
          <button class="btn" type="button" onclick={() => zoom.act("reset")} disabled={zoom.percent === ZOOM_DEFAULT}
            title="Back to 100% (Ctrl and 0)">Normal size</button>
        </div>
        {#if zoom.problem}<p class="alert" role="alert">{zoom.problem.what}: {zoom.problem.error}</p>{/if}
        <p class="hint">
          Changes straight away, without Save, and is kept for next time. Anywhere in the app you can also hold Ctrl
          and press + or - (0 for 100%), or hold Ctrl and turn the mouse wheel.
        </p>
      </div>
    </section>
  </div>

  <div class="save-bar">
    <button class="btn primary" type="submit" disabled={saving || !settings.loaded}>{saving ? "Saving…" : "Save settings"}</button>
    {#if savedNote}<span class="saved" role="status"><Icon path={icons.check} size={15} /> Saved</span>{/if}
    {#if tried && !valid}<span class="problem" role="alert">Some values need fixing before they can be saved.</span>{/if}
    {#if saveError}<span class="alert" role="alert">Not saved: {saveError}</span>{/if}
    {#if info}
      <span class="muted small about">home-cc {info.version} on {info.os}</span>
    {:else if infoError}
      <span class="muted small about">Couldn't read the app's version: {infoError}</span>
    {/if}
  </div>
</form>

<style>
  /* Five panels: one row on the ultrawide, two side by side on the other screens. */
  .grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(min(100%, 34rem), 1fr)); gap: var(--gap); align-items: start; }
  @media (min-width: 1500px) and (max-width: 3000px) {
    .grid { grid-template-columns: repeat(2, minmax(0, 1fr)); }
  }
  .panel { display: flex; flex-direction: column; gap: 0.9rem; }
  /* A message about a field sits right under it. */
  .panel > .problem, .panel > label + .hint { margin-top: -0.55rem; }
  .section-title { margin-bottom: 0; }

  .with-unit { display: flex; align-items: center; gap: 0.6rem; }
  .with-unit input[type="number"] { width: 8rem; }
  .field { display: flex; flex-direction: column; gap: 0.35rem; }
  .field .hint, .field .alert { margin-top: 0.3rem; }
  .hint { margin: 0; font-size: 0.92rem; line-height: 1.5; color: var(--muted); }
  .hint .mono { color: var(--text-soft); font-size: 0.92em; overflow-wrap: anywhere; }
  .problem { color: var(--bad); font-size: 0.92rem; }

  .stored { display: flex; align-items: center; justify-content: space-between; gap: 0.5rem; flex-wrap: wrap; margin-top: 0.3rem; color: var(--text-soft); }
  .stored > span:first-child { display: flex; align-items: center; gap: 0.4rem; }
  .stored :global(svg), .saved :global(svg) { color: var(--ok); }
  .stored-actions { display: flex; gap: 0.4rem; }

  .zoom { display: flex; align-items: center; gap: 0.5rem; flex-wrap: wrap; margin-top: 0.2rem; }
  .zoom .step { min-width: 2.75rem; padding: 0; font-size: 1.15rem; }
  .zoom-now { min-width: 5ch; text-align: center; font-size: 1.05rem; }

  .save-bar { display: flex; align-items: center; gap: 0.6rem 1rem; flex-wrap: wrap; margin-top: var(--gap); }
  .saved { display: flex; align-items: center; gap: 0.35rem; }
  .about { margin-left: auto; }
</style>

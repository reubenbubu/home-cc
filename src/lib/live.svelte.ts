// Wires the stores to the back end: one load at start, then reloads only when
// an event says something changed. Called once from the root layout.
import type { UnlistenFn } from "@tauri-apps/api/event";
import { errorText, events } from "./api";
import { clock } from "./clock.svelte";
import { devices } from "./devices.svelte";
import { scan } from "./scan.svelte";
import { settings } from "./settings.svelte";
import { stats } from "./stats.svelte";
import { status } from "./status.svelte";

class Live {
  /** Set when listening for events failed, so the screens would go stale. */
  error = $state<string | null>(null);
}

export const live = new Live();

/** Returns a function that stops listening. */
export function startLive(): () => void {
  let stopped = false;
  const unlisten: UnlistenFn[] = [];
  const keep = (p: Promise<UnlistenFn>) =>
    p.then(
      (u) => {
        if (stopped) u();
        else unlisten.push(u);
      },
      (e) => {
        live.error = errorText(e);
      },
    );

  keep(events.onScanProgress((p) => scan.onProgress(p)));
  keep(
    events.onStatusChanged(() => {
      devices.load();
      status.load();
      scan.onInventoryChanged();
    }),
  );
  keep(events.onStatsChanged(() => stats.load()));

  devices.load();
  status.load();
  stats.load();
  settings.load();
  scan.init();
  const stopClock = clock.start();

  return () => {
    stopped = true;
    stopClock();
    for (const u of unlisten) u();
  };
}

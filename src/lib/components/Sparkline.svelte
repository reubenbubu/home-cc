<script lang="ts">
  // Recent checks of one device, newest on the right: a line for the response
  // time and a red block wherever the device didn't answer.
  import type { Sample } from "$lib/api";
  import { ms } from "$lib/format";

  let { history, slots = 120 }: { history: Sample[]; slots?: number } = $props();

  const W = 120;
  const H = 20;
  const PAD = 2;

  const view = $derived.by(() => {
    const list = history.slice(-slots);
    const step = W / slots;
    const left = (slots - list.length) * step;
    const x = (i: number) => left + (i + 0.5) * step;
    const times = list.flatMap((s) => (s.up && s.rttMs != null && Number.isFinite(s.rttMs) ? [s.rttMs] : []));
    // Never zoom in closer than 4 ms, so a steady wired device draws as a calm flat line.
    const top = Math.max(4, ...times);
    const y = (rtt: number) => H - PAD - (Math.min(Math.max(rtt, 0), top) / top) * (H - 2 * PAD - 1);

    let line = "";
    let quiet = "";
    const gaps: { x: number; width: number }[] = [];
    let pen = false;
    let missed = 0;
    list.forEach((s, i) => {
      if (!s.up) {
        missed++;
        pen = false;
        const last = gaps[gaps.length - 1];
        const from = left + i * step;
        if (last && Math.abs(last.x + last.width - from) < 0.01) last.width += step;
        else gaps.push({ x: from, width: step });
      } else if (s.rttMs == null || !Number.isFinite(s.rttMs)) {
        // There, but it doesn't answer ping: a flat grey stretch.
        pen = false;
        quiet += `M${(left + i * step).toFixed(2)} ${H - PAD}h${step.toFixed(2)}`;
      } else {
        line += `${pen ? "L" : "M"}${x(i).toFixed(2)} ${y(s.rttMs).toFixed(2)}${pen ? "" : "h0"}`;
        pen = true;
      }
    });

    let label = "No checks yet";
    const first = list[0];
    const last = list[list.length - 1];
    if (first && last) {
      const mins = Math.max(1, Math.round((last.ts - first.ts) / 60));
      const answered = missed === 0 ? "answered every check" : `missed ${missed} of ${list.length} checks`;
      const range =
        times.length > 0 ? `, response ${ms(Math.min(...times))} to ${ms(Math.max(...times))}` : "";
      label = `Last ${mins} min: ${answered}${range}`;
    }
    // A single missed check is one slot wide, which is too thin to notice.
    const blocks = gaps.map((g) => (g.width < 1.8 ? { x: g.x - (1.8 - g.width) / 2, width: 1.8 } : g));
    return { line, quiet, blocks, label };
  });
</script>

<svg class="spark" viewBox="0 0 {W} {H}" preserveAspectRatio="none" role="img" aria-label={view.label}>
  <title>{view.label}</title>
  <path class="base" d="M0 {H - 0.5}H{W}" />
  {#each view.blocks as b}
    <rect class="gap" x={b.x} y={PAD} width={b.width} height={H - PAD - 0.5} />
  {/each}
  {#if view.quiet}<path class="quiet" d={view.quiet} />{/if}
  {#if view.line}<path class="line" d={view.line} />{/if}
</svg>

<style>
  .spark { display: block; width: 100%; height: 1.7rem; overflow: visible; }
  path { fill: none; vector-effect: non-scaling-stroke; }
  .base { stroke: var(--track); stroke-width: 1; }
  .line { stroke: var(--accent); stroke-width: 1.5; stroke-linecap: round; stroke-linejoin: round; }
  .quiet { stroke: var(--idle); stroke-width: 2; }
  .gap { fill: var(--bad); opacity: 0.85; }
</style>

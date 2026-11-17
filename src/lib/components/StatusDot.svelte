<script lang="ts">
  import type { Health } from "$lib/status.svelte";

  let { health, label }: { health: Health; label?: string } = $props();

  const TEXT: Record<Health, string> = { up: "Up", down: "Down", unknown: "Not checked yet" };
  const CLASS: Record<Health, string> = { up: "ok", down: "bad", unknown: "idle" };
  const text = $derived(label ?? TEXT[health]);
</script>

<span class="dot {CLASS[health]}" role="img" aria-label={text} title={text}></span>

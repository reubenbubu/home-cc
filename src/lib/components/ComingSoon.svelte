<script lang="ts">
  let { title, step, items }: { title: string; step: string; items: string[] } = $props();
</script>

<header class="page-head">
  <h1>{title}</h1>
</header>

<section class="panel soon">
  <h2 class="section-title">Coming in {step}</h2>
  <ul>
    {#each items as item}
      <li>{item}</li>
    {/each}
  </ul>
</section>

<style>
  .page-head h1 { margin: 0 0 1.25rem; font-size: 1.55rem; font-weight: 600; }
  .soon { max-width: 46rem; }
  ul { margin: 0; padding-left: 1.2rem; line-height: 1.9; color: var(--text-soft); }
</style>

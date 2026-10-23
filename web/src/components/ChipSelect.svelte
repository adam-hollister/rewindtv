<script lang="ts">
  interface Option { value: string; label: string; count?: number }
  let { options, selected = $bindable(), negative = false, placeholder = 'Search…', limit = 30 }:
    { options: Option[]; selected: string[]; negative?: boolean; placeholder?: string; limit?: number } = $props();

  let query = $state('');
  const sel = $derived(new Set(selected));
  const matches = $derived(
    options.filter((o) => !sel.has(o.value) && (!query || o.label.toLowerCase().includes(query.toLowerCase()))).slice(0, query ? 60 : limit),
  );
  const labelOf = (v: string) => options.find((o) => o.value === v)?.label ?? v;

  function toggle(v: string) {
    selected = sel.has(v) ? selected.filter((x) => x !== v) : [...selected, v];
  }
</script>

<div class="cs">
  {#if selected.length}
    <div class="chips">
      {#each selected as v (v)}
        <button type="button" class="chip on" class:neg={negative} onclick={() => toggle(v)}>{labelOf(v)} <span aria-hidden="true">✕</span></button>
      {/each}
    </div>
  {/if}
  {#if options.length > 12}
    <input type="search" bind:value={query} {placeholder} />
  {/if}
  <div class="chips">
    {#each matches as o (o.value)}
      <button type="button" class="chip" onclick={() => toggle(o.value)}>{o.label}{#if o.count !== undefined}<span class="dim">{o.count}</span>{/if}</button>
    {/each}
    {#if !matches.length && query}<span class="dim small">No matches</span>{/if}
  </div>
</div>

<style>
  .cs { display: grid; gap: 10px; }
  .chips { display: flex; flex-wrap: wrap; gap: 6px; }
  input { height: 36px; max-width: 280px; }
</style>

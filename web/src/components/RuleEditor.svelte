<script lang="ts">
  import { api, type Facets, type Rule, type RulePreview } from '../lib/api';
  import ChipSelect from './ChipSelect.svelte';

  let { rule = $bindable(), facets, compact = false }: { rule: Rule; facets: Facets; compact?: boolean } = $props();

  let preview = $state<RulePreview | null>(null);
  let timer: ReturnType<typeof setTimeout> | undefined;

  $effect(() => {
    const snapshot = JSON.stringify(rule);
    clearTimeout(timer);
    timer = setTimeout(async () => {
      try { preview = await api.previewRule(JSON.parse(snapshot)); } catch { preview = null; }
    }, 350);
    return () => clearTimeout(timer);
  });

  const kind = $derived(rule.kinds.length === 1 ? rule.kinds[0] : 'all');
  function setKind(k: string) {
    rule.kinds = k === 'all' ? [] : [k];
  }

  const decades = $derived.by(() => {
    if (facets.year_min == null || facets.year_max == null) return [];
    const out: number[] = [];
    for (let d = Math.floor(facets.year_min / 10) * 10; d <= facets.year_max; d += 10) out.push(d);
    return out;
  });
  function setDecade(d: number) {
    if (rule.year_min === d && rule.year_max === d + 9) {
      rule.year_min = null;
      rule.year_max = null;
    } else {
      rule.year_min = d;
      rule.year_max = d + 9;
    }
  }

  const opts = (list: { name: string; count: number }[]) => list.map((g) => ({ value: g.name, label: g.name, count: g.count }));
  let more = $state(false);
</script>

<div class="re">
  <div class="match" class:zero={preview?.items === 0}>
    {#if preview}
      <strong>{preview.items.toLocaleString()}</strong> items · <strong>{Math.round(preview.hours).toLocaleString()}</strong> hours
      <span class="dim">({preview.shows} shows, {preview.movies} movies)</span>
      {#if preview.sample.length && !compact}<div class="sample dim small">{preview.sample.join(' · ')}</div>{/if}
    {:else}
      <span class="dim">Counting…</span>
    {/if}
  </div>

  <div class="sec">
    <h4>Type</h4>
    <div class="segmented">
      <button type="button" class:on={kind === 'all'} onclick={() => setKind('all')}>Everything</button>
      <button type="button" class:on={kind === 'episode'} onclick={() => setKind('episode')}>TV episodes</button>
      <button type="button" class:on={kind === 'movie'} onclick={() => setKind('movie')}>Movies</button>
    </div>
  </div>

  <div class="sec">
    <h4>Genres <span class="dim">— any of</span></h4>
    <ChipSelect options={opts(facets.genres)} bind:selected={rule.genres} />
  </div>

  {#if decades.length}
    <div class="sec">
      <h4>Era</h4>
      <div class="chips">
        {#each decades as d}
          <button type="button" class="chip" class:on={rule.year_min === d && rule.year_max === d + 9} onclick={() => setDecade(d)}>{d}s</button>
        {/each}
      </div>
      <div class="row years">
        <input type="number" placeholder="From year" bind:value={rule.year_min} min="1900" max="2100" />
        <span class="dim">to</span>
        <input type="number" placeholder="To year" bind:value={rule.year_max} min="1900" max="2100" />
      </div>
    </div>
  {/if}

  {#if facets.shows.length}
    <div class="sec">
      <h4>Specific shows</h4>
      <ChipSelect options={facets.shows.map((s) => ({ value: s.key, label: s.title, count: s.episodes }))} bind:selected={rule.shows} placeholder="Search shows…" limit={16} />
    </div>
  {/if}

  <button type="button" class="btn small ghost more" onclick={() => (more = !more)}>{more ? '− Fewer filters' : '+ More filters'} <span class="dim">collections, networks, ratings, libraries</span></button>

  {#if more}
    {#if facets.collections.length}
      <div class="sec"><h4>Collections</h4><ChipSelect options={opts(facets.collections)} bind:selected={rule.collections} /></div>
    {/if}
    {#if facets.studios.length}
      <div class="sec"><h4>Networks & studios</h4><ChipSelect options={opts(facets.studios)} bind:selected={rule.studios} /></div>
    {/if}
    {#if facets.ratings.length}
      <div class="sec"><h4>Ratings</h4><ChipSelect options={opts(facets.ratings)} bind:selected={rule.ratings} /></div>
    {/if}
    <div class="sec"><h4>Never play these genres</h4><ChipSelect options={opts(facets.genres)} bind:selected={rule.exclude_genres} negative /></div>
    {#if facets.libraries.length > 1}
      <div class="sec">
        <h4>Libraries</h4>
        <ChipSelect options={facets.libraries.map((l) => ({ value: String(l.id), label: `${l.name} (${l.source})` }))}
          bind:selected={() => rule.libraries.map(String), (v) => (rule.libraries = v.map(Number))} />
      </div>
    {/if}
  {/if}
</div>

<style>
  .re { display: grid; gap: 18px; }
  .match { padding: 12px 14px; border-radius: 12px; background: rgb(52 211 153 / 0.08); border: 1px solid rgb(52 211 153 / 0.3); font-size: 14px; }
  .match.zero { background: rgb(248 113 113 / 0.08); border-color: rgb(248 113 113 / 0.35); }
  .sample { margin-top: 4px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .sec { display: grid; gap: 8px; }
  h4 { margin: 0; font-size: 13px; font-weight: 750; color: var(--text-2); text-transform: uppercase; letter-spacing: 0.06em; }
  h4 .dim { text-transform: none; letter-spacing: 0; font-weight: 500; }
  .chips { display: flex; flex-wrap: wrap; gap: 6px; }
  .years input { max-width: 130px; height: 36px; }
  .more { justify-self: start; }
</style>

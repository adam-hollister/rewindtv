<script lang="ts">
  import { onMount } from 'svelte';
  import { api, type ChannelListItem } from '../lib/api';
  import { session, go, toast, clockState } from '../lib/state.svelte';
  import { progress } from '../lib/time';
  import ChannelBadge from './ChannelBadge.svelte';
  import AutoCreate from './AutoCreate.svelte';

  let channels = $state<ChannelListItem[]>([]);
  let loaded = $state(false);
  let showAuto = $state(false);
  let filter = $state('');

  async function load() {
    channels = await api.channels(session.userId ?? undefined);
    loaded = true;
  }
  onMount(load);

  const shown = $derived(channels.filter((c) => !filter || c.name.toLowerCase().includes(filter.toLowerCase()) || String(c.number) === filter));

  async function toggleHidden(c: ChannelListItem) {
    if (session.userId === null) return;
    await api.setPrefs(session.userId, c.id, { hidden: !c.hidden });
    c.hidden = c.hidden ? 0 : 1;
  }

  async function toggleFav(c: ChannelListItem) {
    if (session.userId === null) return;
    await api.setPrefs(session.userId, c.id, { favorite: !c.favorite });
    c.favorite = c.favorite ? 0 : 1;
  }

  async function remove(c: ChannelListItem) {
    if (!confirm(`Delete channel ${c.number} “${c.name}”? This removes it for everyone.`)) return;
    await api.deleteChannel(c.id);
    toast(`Deleted ${c.name}`);
    load();
  }
</script>

<div class="page">
  <div class="head">
    <div>
      <h1>Channels</h1>
      <p class="muted">Shared channels show up for every profile. Private ones only for you. Hide anything you don't want in your guide.</p>
    </div>
    <div class="row wrap">
      <button class="btn" onclick={() => (showAuto = true)}>✨ Auto-create</button>
      <a class="btn primary" href="#/channels/new">+ New channel</a>
    </div>
  </div>

  {#if loaded && channels.length === 0}
    <div class="empty">
      <div class="big">📺</div>
      <h2>Let's build your lineup</h2>
      <p class="muted">Auto-create turns your genres, decades and favorite shows into channels in one click.</p>
      <div class="row"><button class="btn primary" onclick={() => (showAuto = true)}>✨ Auto-create channels</button><a class="btn" href="#/channels/new">Build one by hand</a></div>
    </div>
  {:else}
    {#if channels.length > 8}
      <input class="search" type="search" placeholder="Find a channel…" bind:value={filter} />
    {/if}
    <div class="grid">
      {#each shown as c (c.id)}
        <article class="card ch" class:hidden={c.hidden} style:--c={c.color}>
          <div class="art">
            {#if c.now?.art || c.now?.thumb}<img src={c.now.art ?? c.now.thumb} alt="" loading="lazy" />{/if}
            <div class="art-shade"></div>
            <ChannelBadge number={c.number} color={c.color} size="lg" />
            <span class="owner badge">{c.owner_user_id === null ? 'Shared' : 'Private'}</span>
          </div>
          <div class="body">
            <div class="row"><h3>{c.name}</h3><div class="spacer"></div>
              <button class="star" class:on={c.favorite} onclick={() => toggleFav(c)} aria-label="Favorite" title="Favorite">★</button>
            </div>
            {#if c.now}
              <div class="now small"><span class="live-dot"></span> {c.now.title}{#if c.now.subtitle}<span class="dim"> · {c.now.subtitle}</span>{/if}</div>
              <div class="bar"><i style:width="{progress(c.now, clockState.now) * 100}%"></i></div>
            {:else}
              <div class="now small dim">Nothing scheduled</div>
            {/if}
            <div class="row actions">
              <button class="btn small primary" onclick={() => go(`watch/${c.id}`)}>▶ Watch</button>
              <button class="btn small" onclick={() => go(`channels/${c.id}`)}>Edit</button>
              <div class="spacer"></div>
              <button class="btn small ghost" onclick={() => toggleHidden(c)} title={c.hidden ? 'Show in my guide' : 'Hide from my guide'}>{c.hidden ? 'Show' : 'Hide'}</button>
              <button class="btn small ghost danger" onclick={() => remove(c)} aria-label="Delete">Delete</button>
            </div>
          </div>
        </article>
      {/each}
    </div>
  {/if}
</div>

{#if showAuto}
  <AutoCreate onclose={() => (showAuto = false)} ondone={() => { showAuto = false; load(); }} />
{/if}

<style>
  .head { display: flex; justify-content: space-between; align-items: flex-end; gap: 20px; margin-bottom: 24px; flex-wrap: wrap; }
  .head p { margin-top: 6px; max-width: 560px; }
  .search { max-width: 320px; margin-bottom: 16px; }
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(290px, 1fr)); gap: 18px; }
  .ch { overflow: hidden; transition: transform 0.15s, border-color 0.15s; }
  .ch:hover { border-color: color-mix(in srgb, var(--c) 50%, var(--line)); }
  .ch.hidden { opacity: 0.5; }
  .art { position: relative; height: 130px; background: linear-gradient(135deg, color-mix(in srgb, var(--c) 35%, #111), #0d1017); display: flex; align-items: flex-end; padding: 14px; }
  .art img { position: absolute; inset: 0; width: 100%; height: 100%; object-fit: cover; opacity: 0.6; }
  .art-shade { position: absolute; inset: 0; background: linear-gradient(0deg, rgb(10 12 17 / 0.9), transparent 70%); }
  .art :global(.badge-ch) { position: relative; }
  .owner { position: absolute; top: 12px; right: 12px; background: rgb(0 0 0 / 0.5); }
  .body { padding: 14px 16px 16px; display: grid; gap: 8px; }
  .now { display: flex; align-items: center; gap: 8px; white-space: nowrap; overflow: hidden; min-width: 0; }
  .now :global(.live-dot) { margin: 0 3px; }
  .actions { margin-top: 6px; gap: 6px; }
  .star { border: 0; background: none; font-size: 18px; color: var(--text-3); }
  .star.on { color: var(--accent); }
  .big { font-size: 48px; }
</style>

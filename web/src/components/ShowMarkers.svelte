<script lang="ts">
  import { onMount } from 'svelte';
  import { api, type EpisodeMarkers, type MarkerOut, type MarkerRule, type ShowMarkers } from '../lib/api';
  import { toast } from '../lib/state.svelte';
  import { mmss, parseMmss, duration } from '../lib/time';
  import MarkerPreview from './MarkerPreview.svelte';

  let { showKey }: { showKey: string } = $props();

  type Kind = 'intro' | 'credits';
  const SOURCE: Record<string, string> = { manual: 'Mine', rule: 'Season rule', server: 'Plex/Jellyfin', detected: 'Detected' };

  let data = $state<ShowMarkers | null>(null);
  let season = $state<number | null>(null);
  let editing = $state<{ ep: EpisodeMarkers; kind: Kind; start: string; end: string } | null>(null);
  let preview = $state<{ ep: EpisodeMarkers; kind: Kind; at: number } | null>(null);

  // Season rule forms.
  let scopeAll = $state(false);
  let introStart = $state('');
  let introEnd = $state('');
  let creditsMode = $state<'time' | 'last'>('last');
  let creditsStart = $state('');
  let creditsEnd = $state('');
  let creditsLast = $state('');

  async function load() {
    data = await api.showMarkers(showKey);
    if (season === null) season = data.episodes[0]?.season ?? 1;
    fillRules();
  }

  function ruleFor(kind: Kind): MarkerRule | undefined {
    const rs = data?.rules ?? [];
    return scopeAll ? rs.find((r) => r.kind === kind && r.season === null) : rs.find((r) => r.kind === kind && r.season === season);
  }

  function fillRules() {
    const i = ruleFor('intro');
    introStart = i ? mmss(i.start_ms) : '';
    introEnd = i ? mmss(i.end_ms) : '';
    const c = ruleFor('credits');
    creditsMode = c && !c.from_end ? 'time' : 'last';
    creditsStart = c && !c.from_end ? mmss(c.start_ms) : '';
    creditsEnd = c && !c.from_end ? mmss(c.end_ms) : '';
    creditsLast = c && c.from_end ? mmss(c.start_ms - c.end_ms) : '';
  }

  const seasons = $derived([...new Set((data?.episodes ?? []).map((e) => e.season))]);
  const episodes = $derived((data?.episodes ?? []).filter((e) => e.season === season));
  const marker = (list: MarkerOut[], kind: Kind) => list.find((m) => m.kind === kind);

  async function saveRule(kind: Kind) {
    let r: MarkerRule;
    if (kind === 'intro') {
      const s = parseMmss(introStart), e = parseMmss(introEnd);
      if (s === null || e === null || e <= s) return toast('Enter the intro start and end, like 1:32 and 3:02', 'err');
      r = { show_key: showKey, season: scopeAll ? null : season, kind, start_ms: s, end_ms: e, from_end: false };
    } else if (creditsMode === 'last') {
      const l = parseMmss(creditsLast);
      if (l === null || l <= 0) return toast('Enter how long the credits run, like 1:30', 'err');
      r = { show_key: showKey, season: scopeAll ? null : season, kind, start_ms: l, end_ms: 0, from_end: true };
    } else {
      const s = parseMmss(creditsStart), e = parseMmss(creditsEnd);
      if (s === null || e === null || e <= s) return toast('Enter the credits start and end', 'err');
      r = { show_key: showKey, season: scopeAll ? null : season, kind, start_ms: s, end_ms: e, from_end: false };
    }
    try {
      await api.saveMarkerRule(r);
      toast(`Saved ${kind} for ${scopeAll ? 'every season' : `season ${season}`}`);
      load();
    } catch (e) {
      toast((e as Error).message, 'err');
    }
  }

  async function removeRule(kind: Kind) {
    await api.deleteMarkerRule(showKey, scopeAll ? null : season, kind);
    toast('Removed');
    load();
  }

  function edit(ep: EpisodeMarkers, kind: Kind) {
    const m = marker(ep.resolved, kind);
    editing = { ep, kind, start: m ? mmss(m.start_ms) : '', end: m ? mmss(m.end_ms) : '' };
  }

  async function saveEpisode() {
    if (!editing) return;
    const s = parseMmss(editing.start), e = parseMmss(editing.end);
    if (s === null || e === null || e <= s) return toast('Enter a start and end, like 1:32 and 3:02', 'err');
    try {
      await api.setEpisodeMarker(editing.ep.media_key, editing.kind, [s, e]);
      toast(`Saved for E${editing.ep.episode}`);
      editing = null;
      load();
    } catch (err) {
      toast((err as Error).message, 'err');
    }
  }

  async function clearEpisode(ep: EpisodeMarkers, kind: Kind) {
    await api.setEpisodeMarker(ep.media_key, kind, null);
    editing = null;
    load();
  }

  function onmark(which: 'start' | 'end', ms: number) {
    if (!preview) return;
    if (!editing || editing.ep !== preview.ep || editing.kind !== preview.kind) edit(preview.ep, preview.kind);
    if (editing) {
      if (which === 'start') editing.start = mmss(ms);
      else editing.end = mmss(ms);
    }
    toast(`${which === 'start' ? 'Start' : 'End'} set to ${mmss(ms)}`);
  }

  onMount(load);
</script>

<div class="page">
  <div class="head">
    <a class="btn ghost small" href="#/settings">← Settings</a>
    <div>
      <h1>{data?.title ?? '…'}</h1>
      <p class="muted small">Intro &amp; credits markers. What you set here is saved with RewindTV's data (not next to the media) and applies on every server that has this show.</p>
    </div>
  </div>

  {#if data}
    {#if seasons.length > 1}
      <div class="chips">
        {#each seasons as s}<button class="chip" class:on={s === season} onclick={() => { season = s; editing = null; fillRules(); }}>Season {s}</button>{/each}
      </div>
    {/if}

    <section class="card pad">
      <div class="row wrap"><h2>Same for every episode</h2><div class="spacer"></div>
        <div class="segmented">
          <button class:on={!scopeAll} onclick={() => { scopeAll = false; fillRules(); }}>Season {season}</button>
          <button class:on={scopeAll} onclick={() => { scopeAll = true; fillRules(); }}>All seasons</button>
        </div>
      </div>
      <p class="muted small sub">Most shows use the same opening and closing in every episode of a season. Set it once here; single-episode markers below still win.</p>
      <div class="rule">
        <b>Intro</b>
        <input type="text" placeholder="start 1:32" bind:value={introStart} />
        <span class="dim">to</span>
        <input type="text" placeholder="end 3:02" bind:value={introEnd} />
        <div class="spacer"></div>
        <button class="btn small primary" onclick={() => saveRule('intro')}>Save</button>
        {#if ruleFor('intro')}<button class="btn small ghost danger" onclick={() => removeRule('intro')}>Remove</button>{/if}
      </div>
      <div class="rule">
        <b>Credits</b>
        <div class="segmented">
          <button class:on={creditsMode === 'last'} onclick={() => (creditsMode = 'last')}>Last …</button>
          <button class:on={creditsMode === 'time'} onclick={() => (creditsMode = 'time')}>At a time</button>
        </div>
        {#if creditsMode === 'last'}
          <input type="text" placeholder="1:30" bind:value={creditsLast} /><span class="dim small">of each episode</span>
        {:else}
          <input type="text" placeholder="start 21:40" bind:value={creditsStart} /><span class="dim">to</span><input type="text" placeholder="end 23:10" bind:value={creditsEnd} />
        {/if}
        <div class="spacer"></div>
        <button class="btn small primary" onclick={() => saveRule('credits')}>Save</button>
        {#if ruleFor('credits')}<button class="btn small ghost danger" onclick={() => removeRule('credits')}>Remove</button>{/if}
      </div>
    </section>

    <section class="card eps">
      {#each episodes as ep (ep.media_key)}
        <div class="ep">
          <span class="num">E{ep.episode}</span>
          <span class="t"><b>{ep.title}</b><span class="tiny dim">{duration(ep.duration_ms)}</span></span>
          {#each ['intro', 'credits'] as const as kind}
            {@const m = marker(ep.resolved, kind)}
            <button class="mk" class:none={!m} onclick={() => edit(ep, kind)} title="Edit {kind}">
              <span class="tiny dim">{kind === 'intro' ? 'Intro' : 'Credits'}</span>
              {#if m}
                <span>{mmss(m.start_ms)}–{mmss(m.end_ms)}</span>
                <span class="src {m.source}">{SOURCE[m.source]}</span>
              {:else}
                <span class="dim">— none</span>
              {/if}
            </button>
          {/each}
        </div>
        {#if editing && editing.ep.media_key === ep.media_key}
          <div class="editor">
            <b>{editing.kind === 'intro' ? 'Intro' : 'Credits'} for E{ep.episode}</b>
            <input type="text" placeholder="start" bind:value={editing.start} />
            <span class="dim">to</span>
            <input type="text" placeholder="end" bind:value={editing.end} />
            <button class="btn small" onclick={() => (preview = { ep, kind: editing!.kind, at: parseMmss(editing!.start) ?? 0 })}>▶ Preview</button>
            <div class="spacer"></div>
            <button class="btn small primary" onclick={saveEpisode}>Save for this episode</button>
            {#if ep.candidates.some((c) => c.kind === editing!.kind && c.source === 'manual')}
              <button class="btn small ghost danger" onclick={() => clearEpisode(ep, editing!.kind)}>Clear mine</button>
            {/if}
            <button class="btn small ghost" onclick={() => (editing = null)}>Cancel</button>
            {#if ep.candidates.filter((c) => c.kind === editing!.kind).length > 1}
              <p class="tiny dim others">Also known: {ep.candidates.filter((c) => c.kind === editing!.kind).map((c) => `${SOURCE[c.source]} ${mmss(c.start_ms)}–${mmss(c.end_ms)}`).join(' · ')}</p>
            {/if}
          </div>
        {/if}
      {/each}
    </section>
  {/if}
</div>

{#if preview}
  <MarkerPreview itemId={preview.ep.item_id} title="E{preview.ep.episode} {preview.ep.title}" at={preview.at} {onmark} onclose={() => (preview = null)} />
{/if}

<style>
  .page { display: grid; gap: 16px; max-width: 1000px; }
  .head { display: flex; gap: 14px; align-items: flex-start; }
  .head p { margin-top: 4px; max-width: 640px; }
  .chips { display: flex; gap: 6px; flex-wrap: wrap; }
  .pad { padding: 20px; }
  .sub { margin: 6px 0 12px; }
  .spacer { flex: 1; }
  .rule, .editor { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; padding: 10px 0; border-top: 1px solid var(--line); }
  .rule b { width: 64px; }
  .rule input, .editor input { width: 110px; height: 36px; }
  .eps { padding: 6px; }
  .ep { display: grid; grid-template-columns: 44px 1fr 190px 190px; gap: 10px; align-items: center; padding: 8px 10px; border-radius: 10px; }
  .ep:hover { background: var(--panel-2); }
  .num { font-family: var(--display); color: var(--text-2); }
  .t { display: grid; min-width: 0; }
  .t b { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .mk { display: grid; grid-template-columns: auto 1fr; column-gap: 8px; text-align: left; padding: 6px 10px; border-radius: 10px; border: 1px solid var(--line); background: var(--bg-2); font-variant-numeric: tabular-nums; font-size: 14px; }
  .mk:hover { border-color: var(--line-2); }
  .mk .tiny { grid-column: span 2; }
  .mk.none { opacity: 0.6; }
  .src { font-size: 10.5px; font-weight: 800; padding: 1px 6px; border-radius: 6px; background: var(--panel-2); color: var(--text-2); justify-self: end; }
  .src.manual { color: var(--accent-2); }
  .src.server { color: #e5a00d; }
  .src.detected { color: #7dd3fc; }
  .src.rule { color: var(--ok); }
  .editor { margin: 0 10px 8px; padding: 12px; border-radius: 12px; background: var(--bg-2); border: 1px solid rgb(245 165 36 / 0.4); }
  .others { width: 100%; margin: 0; }
  @media (max-width: 720px) {
    .ep { grid-template-columns: 40px 1fr; }
    .mk { grid-column: span 2; }
  }
</style>

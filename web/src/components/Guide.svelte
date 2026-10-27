<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { api, type Guide, type GuideChannel, type Program } from '../lib/api';
  import { session, go, clockState, toast } from '../lib/state.svelte';
  import { clock, range, MIN, progress, minutesLeft, duration } from '../lib/time';
  import ChannelBadge from './ChannelBadge.svelte';

  const HOURS = 24;
  const CH_W_DESKTOP = 210;
  const CH_W_MOBILE = 92;

  let guide = $state<Guide | null>(null);
  let error = $state<string | null>(null);
  let zoom = $state(window.innerWidth < 700 ? 5 : 8); // px per minute
  let chW = $state(window.innerWidth < 700 ? CH_W_MOBILE : CH_W_DESKTOP);
  let onlyFav = $state(false);
  let selected = $state<{ ch: GuideChannel; p: Program } | null>(null);
  let scroller: HTMLDivElement | undefined = $state();
  let digits = $state('');
  let digitTimer: ReturnType<typeof setTimeout> | undefined;

  const now = $derived(clockState.now);
  const channels = $derived((guide?.channels ?? []).filter((c) => !onlyFav || c.favorite));
  const from = $derived(guide?.from ?? now);
  const to = $derived(guide?.to ?? now);
  const width = $derived(((to - from) / MIN) * zoom);
  const slots = $derived.by(() => {
    const out: number[] = [];
    for (let t = from; t < to; t += 30 * MIN) out.push(t);
    return out;
  });

  async function load(scrollToNow = false) {
    try {
      const start = Math.floor(Date.now() / (30 * MIN)) * 30 * MIN - 30 * MIN;
      guide = await api.guide(session.userId ?? undefined, start, HOURS);
      error = null;
      if (!selected && guide.channels.length) {
        const ch = guide.channels[0];
        const p = ch.programs.find((p) => p.start <= Date.now() && p.end > Date.now());
        if (p) selected = { ch, p };
      } else if (selected) {
        const ch = guide.channels.find((c) => c.id === selected!.ch.id);
        const p = ch?.programs.find((p) => p.id === selected!.p.id);
        selected = ch && p ? { ch, p } : null;
      }
      if (scrollToNow) {
        await tick();
        scrollNow();
      }
    } catch (e) {
      error = (e as Error).message;
    }
  }

  function scrollNow() {
    if (!scroller || !guide) return;
    scroller.scrollLeft = Math.max(0, ((Date.now() - guide.from) / MIN) * zoom - 40);
  }

  onMount(() => {
    load(true);
    const t = setInterval(() => load(), 60_000);
    const onResize = () => (chW = window.innerWidth < 700 ? CH_W_MOBILE : CH_W_DESKTOP);
    window.addEventListener('resize', onResize);
    return () => {
      clearInterval(t);
      window.removeEventListener('resize', onResize);
    };
  });

  function geom(p: Program) {
    const s = Math.max(p.start, from);
    const e = Math.min(p.end, to);
    return { left: ((s - from) / MIN) * zoom, width: Math.max(0, ((e - s) / MIN) * zoom - 3) };
  }

  function watch(ch: { id: number }) {
    go(`watch/${ch.id}`);
  }

  function cellKey(e: KeyboardEvent, ch: GuideChannel, p: Program, ri: number, pi: number) {
    let target: HTMLElement | null = null;
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      watch(ch);
      return;
    }
    if (e.key === 'ArrowRight' || e.key === 'ArrowLeft') {
      const next = e.key === 'ArrowRight' ? pi + 1 : pi - 1;
      target = document.querySelector(`[data-cell="${ri}-${next}"]`);
      if (!target && e.key === 'ArrowLeft') target = document.querySelector(`[data-ch="${ri}"]`);
    } else if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      const r = e.key === 'ArrowDown' ? ri + 1 : ri - 1;
      const row = channels[r];
      if (!row) return; // let spatial nav leave the grid
      const t = Math.max(p.start, Math.min(now, p.end - 1), from);
      const idx = row.programs.findIndex((q) => q.start <= t && q.end > t);
      target = document.querySelector(`[data-cell="${r}-${Math.max(0, idx)}"]`);
    }
    if (target) {
      e.preventDefault();
      target.focus();
      target.scrollIntoView({ block: 'nearest', inline: 'nearest' });
    }
  }

  function chKey(e: KeyboardEvent, ri: number) {
    if (e.key === 'ArrowRight') {
      const row = channels[ri];
      const idx = row.programs.findIndex((q) => q.start <= now && q.end > now);
      const el = document.querySelector<HTMLElement>(`[data-cell="${ri}-${Math.max(0, idx)}"]`);
      if (el) {
        e.preventDefault();
        el.focus();
        el.scrollIntoView({ block: 'nearest', inline: 'nearest' });
      }
    }
  }

  // Type a channel number anywhere on the guide to tune it, like a real TV.
  function onKey(e: KeyboardEvent) {
    const t = e.target as HTMLElement;
    if (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA') return;
    if (/^[0-9]$/.test(e.key)) {
      digits = (digits + e.key).slice(-4);
      clearTimeout(digitTimer);
      digitTimer = setTimeout(() => {
        const ch = guide?.channels.find((c) => c.number === Number(digits));
        if (ch) watch(ch);
        else toast(`No channel ${digits}`, 'err');
        digits = '';
      }, 1300);
    }
  }

  async function toggleFav(ch: GuideChannel) {
    if (session.userId === null) return;
    await api.setPrefs(session.userId, ch.id, { favorite: !ch.favorite });
    ch.favorite = ch.favorite ? 0 : 1;
  }

  const sel = $derived(selected);
  const selLive = $derived(sel ? sel.p.start <= now && sel.p.end > now : false);
</script>

<svelte:window onkeydown={onKey} />

<section class="hero" style:--c={sel?.ch.color ?? 'var(--accent)'}>
  {#if sel?.p.art}
    {#key sel.p.art}<img class="backdrop" src={sel.p.art} alt="" />{/key}
  {/if}
  <div class="shade"></div>
  <div class="hero-inner">
    {#if sel}
      {#if sel.p.thumb}
        <img class="poster" src={sel.p.thumb} alt="" />
      {/if}
      <div class="info">
        <div class="kicker">
          <span class="chnum" style:background={sel.ch.color}>{sel.ch.number}</span>
          <span>{sel.ch.name}</span>
          {#if sel.p.block}<span class="badge">{sel.p.block}</span>{/if}
          {#if selLive}<span class="badge live"><span class="live-dot"></span> LIVE</span>{/if}
        </div>
        <h1>{sel.p.title}</h1>
        {#if sel.p.subtitle}<div class="sub">{sel.p.subtitle}</div>{/if}
        <div class="meta">
          <span>{range(sel.p.start, sel.p.end)}</span>
          <span>{duration(sel.p.end - sel.p.start)}</span>
          {#if sel.p.rating}<span class="badge">{sel.p.rating}</span>{/if}
          {#if sel.p.year && sel.p.kind === 'episode'}<span>{sel.p.year}</span>{/if}
          {#if sel.p.trimmed}<span class="badge" title="This channel skips intros/credits">✂ intro/credits skipped</span>{/if}
        </div>
        {#if sel.p.summary}<p class="summary">{sel.p.summary}</p>{/if}
        <div class="row actions">
          <button class="btn primary" onclick={() => watch(sel.ch)}>
            <svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor"><path d="M7 4v16l13-8z" /></svg>
            {selLive ? 'Watch live' : `Tune to ${sel.ch.number}`}
          </button>
          {#if selLive}
            <div class="prog"><div class="bar"><i style:width="{progress(sel.p, now) * 100}%"></i></div><span class="small muted">{minutesLeft(sel.p.end, now)}</span></div>
          {:else if sel.p.start > now}
            <span class="muted small">Starts at {clock(sel.p.start)}</span>
          {/if}
        </div>
      </div>
    {:else if guide && guide.channels.length === 0}
      <div class="info">
        <h1>No channels yet</h1>
        <p class="summary">Connect Plex or Jellyfin in Settings, then create channels — or let RewindTV build a lineup for you.</p>
        <div class="row actions">
          <a class="btn primary" href="#/channels">Create channels</a>
          <a class="btn" href="#/settings">Settings</a>
        </div>
      </div>
    {/if}
  </div>
</section>

<div class="toolbar">
  <div class="segmented">
    <button class:on={!onlyFav} onclick={() => (onlyFav = false)}>All<span class="wide"> channels</span></button>
    <button class:on={onlyFav} onclick={() => (onlyFav = true)}>★<span class="wide"> Favorites</span></button>
  </div>
  <div class="spacer"></div>
  <button class="btn small" onclick={scrollNow}>Now</button>
  <div class="segmented zoom" aria-label="Zoom">
    <button onclick={() => (zoom = Math.max(3, zoom - 2))} aria-label="Zoom out">−</button>
    <button onclick={() => (zoom = Math.min(16, zoom + 2))} aria-label="Zoom in">+</button>
  </div>
</div>

{#if error}
  <div class="page"><div class="empty"><h2>Couldn't load the guide</h2><p class="muted">{error}</p><button class="btn" onclick={() => load(true)}>Retry</button></div></div>
{:else if guide}
  <div class="grid" bind:this={scroller} style:--chw="{chW}px">
    <div class="inner" style:width="{width + chW}px">
      <div class="ruler">
        <div class="corner">{new Date(from).toLocaleDateString(undefined, { weekday: 'short', month: 'short', day: 'numeric' })}</div>
        <div class="times" style:width="{width}px">
          {#each slots as t}
            <div class="tick" style:left="{((t - from) / MIN) * zoom}px" style:width="{30 * zoom}px">
              {clock(t)}
            </div>
          {/each}
        </div>
      </div>

      {#each channels as ch, ri (ch.id)}
        <div class="row-ch">
          <button class="ch" data-ch={ri} onclick={() => watch(ch)} onkeydown={(e) => chKey(e, ri)} title="Watch {ch.name}">
            <ChannelBadge number={ch.number} color={ch.color} />
            <span class="chname">{ch.name}</span>
            <span
              class="fav"
              class:on={ch.favorite}
              role="button"
              tabindex="-1"
              onclick={(e) => { e.stopPropagation(); toggleFav(ch); }}
              onkeydown={() => {}}
              aria-label="Favorite">★</span>
          </button>
          <div class="lane" style:width="{width}px">
            {#each ch.programs as p, pi (p.id)}
              {@const g = geom(p)}
              {@const live = p.start <= now && p.end > now}
              {@const past = p.end <= now}
              {#if g.width > 0}
                <button
                  class="cell"
                  class:live
                  class:past
                  class:filler={p.filler}
                  class:sel={sel?.p.id === p.id}
                  class:tiny={g.width < 44}
                  data-cell="{ri}-{pi}"
                  style:left="{g.left}px"
                  style:width="{g.width}px"
                  style:--c={ch.color}
                  title="{p.title}{p.subtitle ? ' — ' + p.subtitle : ''} ({range(p.start, p.end)})"
                  onclick={() => (selected = { ch, p })}
                  ondblclick={() => watch(ch)}
                  onfocus={() => (selected = { ch, p })}
                  onkeydown={(e) => cellKey(e, ch, p, ri, pi)}>
                  {#if live}<i class="fill" style:width="{progress(p, now) * 100}%"></i>{/if}
                  {#if g.width >= (p.filler ? 130 : 44)}
                    <span class="t">{p.title}</span>
                    {#if g.width >= 110}
                      <span class="s">{p.filler ? range(p.start, p.end) : p.subtitle ?? range(p.start, p.end)}</span>
                    {/if}
                  {/if}
                </button>
              {/if}
            {/each}
          </div>
        </div>
      {/each}

      {#if now >= from && now < to}
        <div class="nowline" style:left="{chW + ((now - from) / MIN) * zoom}px"><span>{clock(now, false)}</span></div>
      {/if}
    </div>
  </div>
  {#if channels.length === 0 && guide.channels.length > 0}
    <div class="page"><div class="empty"><p class="muted">No favorites yet — tap ★ next to a channel.</p></div></div>
  {/if}
{/if}

{#if digits}
  <div class="digits">{digits}</div>
{/if}

<style>
  .hero { position: relative; min-height: 320px; overflow: hidden; border-bottom: 1px solid var(--line); background: #0d1017; }
  .backdrop { position: absolute; inset: 0; width: 100%; height: 100%; object-fit: cover; object-position: center 25%; opacity: 0.55; animation: fade 0.5s ease; }
  @keyframes fade { from { opacity: 0; } }
  .shade { position: absolute; inset: 0; background:
      linear-gradient(90deg, var(--bg) 0%, rgb(10 12 17 / 0.92) 35%, rgb(10 12 17 / 0.35) 75%, rgb(10 12 17 / 0.6) 100%),
      linear-gradient(0deg, var(--bg) 0%, transparent 45%),
      radial-gradient(600px 300px at 0% 100%, color-mix(in srgb, var(--c) 22%, transparent), transparent); }
  .hero-inner { position: relative; display: flex; gap: 28px; align-items: flex-end; padding: 36px 32px 28px; max-width: 1280px; min-height: 320px; }
  .poster { width: 150px; aspect-ratio: 2/3; object-fit: cover; border-radius: 12px; box-shadow: var(--shadow); border: 1px solid rgb(255 255 255 / 0.08); flex: none; }
  .info { display: grid; gap: 10px; max-width: 720px; min-width: 0; }
  .kicker { display: flex; align-items: center; gap: 10px; font-weight: 700; color: var(--text-2); font-size: 14px; flex-wrap: wrap; }
  .chnum { font-family: var(--display); font-size: 13px; color: #111; padding: 2px 8px; border-radius: 6px; }
  .badge.live { color: #fecaca; border-color: rgb(255 77 77 / 0.5); gap: 6px; }
  .hero h1 { font-size: clamp(26px, 3.6vw, 44px); line-height: 1.05; font-weight: 850; text-wrap: balance; }
  .sub { font-size: 17px; font-weight: 600; color: var(--text); opacity: 0.9; overflow-wrap: anywhere; }
  .meta { display: flex; gap: 14px; align-items: center; color: var(--text-2); font-size: 14px; font-weight: 600; flex-wrap: wrap; }
  .summary { color: var(--text-2); display: -webkit-box; -webkit-line-clamp: 2; line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; max-width: 640px; }
  .actions { margin-top: 6px; gap: 18px; }
  .prog { display: grid; gap: 6px; width: 200px; }

  .toolbar { position: sticky; top: 64px; z-index: 12; display: flex; align-items: center; gap: 10px; padding: 12px 24px; background: rgb(10 12 17 / 0.9); backdrop-filter: blur(10px); }
  .zoom button { width: 34px; padding: 0; font-size: 18px; }

  .grid { position: relative; overflow: auto; max-height: calc(100vh - 64px - 58px); scrollbar-width: thin; }
  .inner { position: relative; }
  .ruler { position: sticky; top: 0; z-index: 6; display: flex; height: 40px; background: var(--bg); border-bottom: 1px solid var(--line); }
  .corner { position: sticky; left: 0; z-index: 7; width: var(--chw); flex: none; display: flex; align-items: center; padding-left: 20px; font-size: 13px; font-weight: 700; color: var(--text-2); background: var(--bg); border-right: 1px solid var(--line); }
  .times { position: relative; flex: none; }
  .tick { position: absolute; top: 0; height: 40px; display: flex; align-items: center; padding-left: 10px; font-size: 13px; font-weight: 650; color: var(--text-2); border-left: 1px solid var(--line); white-space: nowrap; font-variant-numeric: tabular-nums; }

  .row-ch { display: flex; height: 72px; border-bottom: 1px solid rgb(38 44 59 / 0.6); }
  .ch {
    position: sticky; left: 0; z-index: 5; width: var(--chw); flex: none;
    display: flex; align-items: center; gap: 12px; padding: 0 12px 0 16px; text-align: left;
    background: linear-gradient(90deg, var(--bg-2), var(--bg-2) 85%, #0f1219); border: 0; border-right: 1px solid var(--line); border-radius: 0;
  }
  .ch:hover { background: var(--panel); }
  .ch:focus-visible { outline-offset: -3px; }
  .chname { font-weight: 700; font-size: 14px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; flex: 1; }
  .fav { color: var(--text-3); font-size: 15px; opacity: 0; transition: opacity 0.15s; }
  .ch:hover .fav, .fav.on { opacity: 1; }
  .fav.on { color: var(--accent); }
  .lane { position: relative; flex: none; }

  .cell {
    position: absolute; top: 6px; bottom: 6px; overflow: hidden;
    display: flex; flex-direction: column; justify-content: center; gap: 2px; padding: 0 12px;
    text-align: left; border: 1px solid var(--line); border-radius: 10px;
    background: var(--panel); transition: background 0.12s, border-color 0.12s, transform 0.12s;
  }
  .cell:hover { background: var(--panel-2); border-color: var(--line-2); }
  .cell.live { background: color-mix(in srgb, var(--c) 13%, var(--panel)); border-color: color-mix(in srgb, var(--c) 45%, var(--line)); }
  .cell.past { opacity: 0.45; }
  .cell.filler { background: repeating-linear-gradient(135deg, #12151d 0 8px, #151924 8px 16px); color: var(--text-3); }
  .cell.sel { box-shadow: inset 0 0 0 2px var(--text); }
  .cell:focus-visible { outline: 3px solid var(--accent); outline-offset: 1px; z-index: 2; }
  .cell.tiny { padding: 0; }
  .fill { position: absolute; left: 0; top: 0; bottom: 0; background: color-mix(in srgb, var(--c) 16%, transparent); pointer-events: none; }
  .cell.live::after { content: ''; position: absolute; left: 0; top: 0; bottom: 0; width: 3px; background: var(--c); }
  .t { position: relative; font-weight: 700; font-size: 14px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .s { position: relative; font-size: 12.5px; color: var(--text-2); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }

  .nowline { position: absolute; top: 0; bottom: 0; width: 2px; background: var(--live); z-index: 4; pointer-events: none; box-shadow: 0 0 12px rgb(255 77 77 / 0.6); }
  .nowline span { position: sticky; top: 8px; display: block; transform: translateX(-50%); width: max-content; margin-left: 1px; padding: 2px 6px; border-radius: 6px; background: var(--live); color: #fff; font-size: 11px; font-weight: 800; }

  .digits { position: fixed; top: 84px; right: 32px; z-index: 50; font-family: var(--display); font-size: 64px; color: var(--accent); text-shadow: 0 4px 24px rgb(0 0 0 / 0.8); }

  .toolbar :global(.segmented button) { white-space: nowrap; }
  @media (max-width: 700px) {
    .wide { display: none; }
    .hero, .hero-inner { min-height: 0; }
    .hero h1 { font-size: 26px; }
    .sub { font-size: 15px; }
    .actions { flex-wrap: wrap; gap: 12px; }
    .prog { width: 140px; }
    .hero-inner { padding: 20px 16px; }
    .poster { width: 84px; }
    .summary { display: none; }
    .toolbar { padding: 10px 12px; }
    .row-ch { height: 64px; }
    .chname { display: none; }
    .ch { justify-content: center; padding: 0; }
    .fav { display: none; }
    .corner { padding-left: 8px; font-size: 11px; }
  }
</style>

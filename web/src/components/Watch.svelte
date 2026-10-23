<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import Hls from 'hls.js';
  import { api, type ChannelListItem, type Tune } from '../lib/api';
  import { session, go, clockState } from '../lib/state.svelte';
  import { clock, range, progress, minutesLeft } from '../lib/time';
  import ChannelBadge from './ChannelBadge.svelte';
  import Static from './Static.svelte';

  let { channelId }: { channelId: number } = $props();

  let video: HTMLVideoElement;
  let hls: Hls | null = null;
  let tune = $state<Tune | null>(null);
  let lineup = $state<ChannelListItem[]>([]);
  let showStatic = $state(true);
  let overlay = $state(true);
  let miniGuide = $state(false);
  let needsTap = $state(false);
  let muted = $state(false);
  let error = $state<string | null>(null);
  let buffering = $state(true);
  let digits = $state('');
  let sessionId: string | null = null;
  let endTimer: ReturnType<typeof setTimeout> | undefined;
  let overlayTimer: ReturnType<typeof setTimeout> | undefined;
  let digitTimer: ReturnType<typeof setTimeout> | undefined;
  let retries = 0;
  let destroyed = false;

  const now = $derived(clockState.now);
  const visible = $derived(lineup.filter((c) => !c.hidden));
  const channel = $derived(lineup.find((c) => c.id === channelId));
  const index = $derived(visible.findIndex((c) => c.id === channelId));

  function poke(ms = 5000) {
    overlay = true;
    clearTimeout(overlayTimer);
    overlayTimer = setTimeout(() => {
      if (!miniGuide) overlay = false;
    }, ms);
  }

  /** Total length of the first variant playlist, to see whether the server
   *  started at our offset or returned the whole program. */
  async function playlistSeconds(url: string): Promise<number> {
    try {
      let text = await (await fetch(url)).text();
      if (text.includes('#EXT-X-STREAM-INF')) {
        const lines = text.split('\n');
        const i = lines.findIndex((l) => l.startsWith('#EXT-X-STREAM-INF'));
        const variant = lines.slice(i + 1).find((l) => l.trim() && !l.startsWith('#'));
        if (!variant) return 0;
        text = await (await fetch(new URL(variant.trim(), new URL(url, location.href)).toString())).text();
      }
      return [...text.matchAll(/#EXTINF:([\d.]+)/g)].reduce((s, m) => s + parseFloat(m[1]), 0);
    } catch {
      return 0;
    }
  }

  function teardown() {
    clearTimeout(endTimer);
    if (hls) {
      hls.destroy();
      hls = null;
    }
  }

  async function start(prevSession: string | null) {
    teardown();
    error = null;
    buffering = true;
    let t: Tune;
    try {
      t = await api.tune(channelId, prevSession);
    } catch (e) {
      error = (e as Error).message;
      return;
    }
    if (destroyed) {
      if (t.session) api.stopSession(t.session);
      return;
    }
    tune = t;
    sessionId = t.session;
    const tunedAt = Date.now();
    // Retune when this program ends (the guide decides what's next).
    endTimer = setTimeout(() => start(sessionId), Math.max(1000, t.program.end - Date.now() + 400));

    if (t.kind !== 'stream' || !t.url) {
      buffering = false;
      video.removeAttribute('src');
      video.load();
      return;
    }

    const remaining = t.duration - t.offset;
    const total = await playlistSeconds(t.url);
    // Did the server start at our offset (playlist ≈ remaining time) or hand
    // back the whole program (playlist ≈ full duration)? Pick the closer one.
    const fullLength = total > 0 && Math.abs(total - t.duration) < Math.abs(total - remaining);
    const pos = () => (fullLength ? t.offset + (Date.now() - tunedAt) / 1000 : 0);

    if (Hls.isSupported()) {
      hls = new Hls({ startPosition: pos(), maxBufferLength: 30, backBufferLength: 60, enableWorker: true });
      hls.on(Hls.Events.ERROR, (_e, data) => {
        if (!data.fatal) return;
        if (data.type === Hls.ErrorTypes.MEDIA_ERROR) {
          hls?.recoverMediaError();
        } else if (retries++ < 2) {
          setTimeout(() => !destroyed && start(sessionId), 2500);
        } else {
          error = 'The media server stopped sending video.';
        }
      });
      hls.loadSource(t.url);
      hls.attachMedia(video);
    } else if (video.canPlayType('application/vnd.apple.mpegurl')) {
      video.src = t.url;
      video.addEventListener('loadedmetadata', () => fullLength && (video.currentTime = pos()), { once: true });
    } else {
      error = 'This browser cannot play HLS video.';
      return;
    }
    play();
  }

  async function play() {
    try {
      video.muted = muted;
      await video.play();
      needsTap = false;
    } catch {
      // Autoplay with sound was blocked; start muted and offer to unmute.
      muted = true;
      video.muted = true;
      needsTap = true;
      video.play().catch(() => {});
    }
  }

  function unmute() {
    muted = false;
    video.muted = false;
    needsTap = false;
    video.play().catch(() => {});
  }

  function change(delta: number) {
    if (!visible.length) return;
    const i = index < 0 ? 0 : (index + delta + visible.length) % visible.length;
    go(`watch/${visible[i].id}`);
  }

  function tuneNumber(n: number) {
    const ch = visible.find((c) => c.number === n) ?? lineup.find((c) => c.number === n);
    if (ch) go(`watch/${ch.id}`);
  }

  function lastChannel() {
    try {
      const last = sessionStorage.getItem('rewindtv.last');
      if (last && Number(last) !== channelId) go(`watch/${last}`);
    } catch { /* ignore */ }
  }

  function toggleFullscreen() {
    if (document.fullscreenElement) document.exitFullscreen();
    else document.documentElement.requestFullscreen?.().catch(() => {});
  }

  function onKey(e: KeyboardEvent) {
    if (e.ctrlKey || e.metaKey || e.altKey) return;
    if (miniGuide) {
      if (e.key === 'Escape' || e.key === 'Backspace' || e.key === 'g') {
        miniGuide = false;
        poke();
        e.preventDefault();
      }
      return; // arrows go to the list (spatial nav)
    }
    if (/^[0-9]$/.test(e.key)) {
      digits = (digits + e.key).slice(-4);
      clearTimeout(digitTimer);
      digitTimer = setTimeout(() => {
        tuneNumber(Number(digits));
        digits = '';
      }, 1300);
      return;
    }
    switch (e.key) {
      case 'ArrowUp':
      case 'PageUp':
      case 'ChannelUp':
      case '+':
        change(1);
        break;
      case 'ArrowDown':
      case 'PageDown':
      case 'ChannelDown':
      case '-':
        change(-1);
        break;
      case 'Enter':
        if (digits) {
          clearTimeout(digitTimer);
          tuneNumber(Number(digits));
          digits = '';
        } else poke();
        break;
      case 'ArrowLeft':
      case 'ArrowRight':
      case 'i':
        poke();
        break;
      case 'g':
      case 'ContextMenu':
        openGuide();
        break;
      case 'l':
        lastChannel();
        break;
      case 'm':
        muted ? unmute() : ((muted = true), (video.muted = true));
        break;
      case 'f':
        toggleFullscreen();
        break;
      case 'Escape':
      case 'Backspace':
      case 'BrowserBack':
        go('guide');
        break;
      default:
        return;
    }
    e.preventDefault();
  }

  function openGuide() {
    miniGuide = true;
    overlay = true;
    clearTimeout(overlayTimer);
    setTimeout(() => document.querySelector<HTMLElement>('.mg .on')?.focus(), 30);
  }

  // Swipe up/down to change channel on phones.
  let touchY = 0;
  function touchstart(e: TouchEvent) {
    touchY = e.touches[0].clientY;
  }
  function touchend(e: TouchEvent) {
    const dy = e.changedTouches[0].clientY - touchY;
    if (Math.abs(dy) > 80 && !miniGuide) change(dy < 0 ? 1 : -1);
  }

  onMount(() => {
    api.channels(session.userId ?? undefined).then((l) => (lineup = l));
    setTimeout(() => (showStatic = false), 450);
    poke(6000);
    start(null);
    try {
      const cur = sessionStorage.getItem('rewindtv.current');
      if (cur && Number(cur) !== channelId) sessionStorage.setItem('rewindtv.last', cur);
      sessionStorage.setItem('rewindtv.current', String(channelId));
    } catch { /* ignore */ }
    const refresh = setInterval(() => api.channels(session.userId ?? undefined).then((l) => (lineup = l)), 60_000);
    return () => clearInterval(refresh);
  });

  onDestroy(() => {
    destroyed = true;
    teardown();
    clearTimeout(overlayTimer);
    clearTimeout(digitTimer);
    if (sessionId) api.stopSession(sessionId);
  });

  const p = $derived(tune?.program);
  const next = $derived(tune?.next);
</script>

<svelte:window onkeydown={onKey} />

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions (keys are handled on window) -->
<div
  class="tv"
  class:hide-cursor={!overlay}
  role="application"
  aria-label="Live TV"
  onmousemove={() => poke()}
  onclick={() => (needsTap ? unmute() : poke())}
  ontouchstart={touchstart}
  ontouchend={touchend}
>
  <!-- svelte-ignore a11y_media_has_caption -->
  <video bind:this={video} playsinline onwaiting={() => (buffering = true)} onplaying={() => (buffering = false)} onended={() => start(sessionId)}></video>

  {#if tune?.kind === 'filler' && p}
    <div class="slate">
      <div class="scan"></div>
      <div class="slate-card">
        {#if channel}<ChannelBadge number={channel.number} color={channel.color} size="lg" />{/if}
        <div class="slate-kicker">COMING UP NEXT</div>
        {#if next}
          {#if next.thumb}<img src={next.thumb} alt="" />{/if}
          <div class="slate-title">{next.title}</div>
          {#if next.subtitle}<div class="muted">{next.subtitle}</div>{/if}
          <div class="slate-time">at {clock(next.start)}</div>
        {:else}
          <div class="muted">Nothing scheduled — check this channel's rules.</div>
        {/if}
      </div>
    </div>
  {/if}

  {#if buffering && tune?.kind === 'stream' && !showStatic}
    <div class="spinner" aria-label="Loading"></div>
  {/if}

  {#if showStatic}
    <Static />
  {/if}

  {#if channel && (showStatic || overlay)}
    <div class="osd-num">{channel.number}</div>
  {/if}

  {#if digits}
    <div class="osd-num entry">{digits}<span>_</span></div>
  {/if}

  {#if needsTap}
    <button class="btn primary unmute" onclick={(e) => { e.stopPropagation(); unmute(); }}>🔊 Tap for sound</button>
  {/if}

  {#if error}
    <div class="slate">
      <div class="slate-card">
        <div class="slate-kicker">TECHNICAL DIFFICULTIES</div>
        <div class="muted">{error}</div>
        <button class="btn" onclick={(e) => { e.stopPropagation(); retries = 0; start(sessionId); }}>Try again</button>
      </div>
    </div>
  {/if}

  <div class="top" class:on={overlay}>
    <button class="btn ghost" onclick={(e) => { e.stopPropagation(); go('guide'); }}>← Guide</button>
    <div class="spacer"></div>
    <span class="clock">{clock(now)}</span>
  </div>

  {#if p && channel}
    <div class="banner" class:on={overlay} role="presentation" onclick={(e) => e.stopPropagation()}>
      <div class="b-left">
        <ChannelBadge number={channel.number} color={channel.color} size="xl" />
        <div class="b-chname">{channel.name}</div>
      </div>
      <div class="b-main">
        <div class="b-kicker">
          <span class="live-dot"></span> LIVE
          {#if p.block}<span class="badge">{p.block}</span>{/if}
          {#if p.rating}<span class="badge">{p.rating}</span>{/if}
        </div>
        <div class="b-title">{p.title}</div>
        {#if p.subtitle}<div class="b-sub">{p.subtitle}</div>{/if}
        <div class="b-prog">
          <span class="tiny">{clock(p.start, false)}</span>
          <div class="bar"><i style:width="{progress(p, now) * 100}%"></i></div>
          <span class="tiny">{clock(p.end, false)}</span>
          <span class="tiny muted">{minutesLeft(p.end, now)}</span>
        </div>
        {#if next}
          <div class="b-next"><span class="dim">UP NEXT</span> {next.title}{next.subtitle && !next.filler ? ` · ${next.subtitle}` : ''} <span class="dim">{clock(next.start)}</span></div>
        {/if}
      </div>
      <div class="b-ctrl">
        <button class="btn icon" onclick={() => change(1)} aria-label="Channel up" title="Channel up (↑)">▲</button>
        <button class="btn icon" onclick={() => change(-1)} aria-label="Channel down" title="Channel down (↓)">▼</button>
        <button class="btn icon" onclick={openGuide} aria-label="Channel list" title="Channel list (G)">☰</button>
        <button class="btn icon" onclick={() => (muted ? unmute() : ((muted = true), (video.muted = true)))} aria-label="Mute" title="Mute (M)">{muted ? '🔇' : '🔊'}</button>
        <button class="btn icon" onclick={toggleFullscreen} aria-label="Fullscreen" title="Fullscreen (F)">⛶</button>
      </div>
    </div>
  {/if}

  {#if miniGuide}
    <div class="mg" role="dialog" aria-label="Channels" onclick={(e) => e.stopPropagation()} onkeydown={() => {}} tabindex="-1">
      <div class="mg-head"><h3>Channels</h3><button class="btn small ghost" onclick={() => (miniGuide = false)}>Close</button></div>
      <div class="mg-list">
        {#each visible as c (c.id)}
          <button class="mg-row" class:on={c.id === channelId} onclick={() => { miniGuide = false; go(`watch/${c.id}`); }}>
            <ChannelBadge number={c.number} color={c.color} />
            <span class="mg-info">
              <span class="mg-name">{c.name}</span>
              <span class="mg-now">{c.now?.title ?? '—'}</span>
              {#if c.now}<span class="bar"><i style:width="{progress(c.now, now) * 100}%"></i></span>{/if}
            </span>
            {#if c.now?.thumb}<img src={c.now.thumb} alt="" />{/if}
          </button>
        {/each}
      </div>
    </div>
  {/if}
</div>

<style>
  .tv { position: fixed; inset: 0; background: #000; overflow: hidden; user-select: none; }
  .tv.hide-cursor { cursor: none; }
  video { position: absolute; inset: 0; width: 100%; height: 100%; object-fit: contain; background: #000; }

  .top { position: absolute; top: 0; left: 0; right: 0; display: flex; align-items: center; gap: 12px; padding: 16px 20px; background: linear-gradient(180deg, rgb(0 0 0 / 0.7), transparent); opacity: 0; transition: opacity 0.3s; }
  .top.on { opacity: 1; }
  .spacer { flex: 1; }
  .clock { font-weight: 750; font-size: 18px; font-variant-numeric: tabular-nums; text-shadow: 0 2px 8px #000; margin-right: 110px; }

  .osd-num { position: absolute; top: 18px; right: 28px; font-family: var(--display); font-size: clamp(40px, 6vw, 72px); color: #fff; text-shadow: 0 0 2px #000, 0 4px 18px rgb(0 0 0 / 0.9); letter-spacing: 0.04em; pointer-events: none; }
  .osd-num.entry { color: var(--accent); }
  .osd-num span { animation: blink 1s steps(2) infinite; }
  @keyframes blink { 50% { opacity: 0; } }

  .banner {
    position: absolute; left: 24px; right: 24px; bottom: 24px;
    display: flex; gap: 24px; align-items: center; padding: 20px 24px;
    background: linear-gradient(180deg, rgb(21 25 34 / 0.78), rgb(12 14 20 / 0.9));
    backdrop-filter: blur(18px) saturate(1.3); border: 1px solid rgb(255 255 255 / 0.08); border-radius: 22px;
    box-shadow: var(--shadow);
    transform: translateY(30px); opacity: 0; transition: transform 0.3s ease, opacity 0.3s ease; pointer-events: none;
  }
  .banner.on { transform: none; opacity: 1; pointer-events: auto; }
  .b-left { display: grid; justify-items: center; gap: 8px; width: 120px; flex: none; }
  .b-chname { font-weight: 750; font-size: 13px; text-align: center; color: var(--text-2); text-transform: uppercase; letter-spacing: 0.06em; }
  .b-main { flex: 1; min-width: 0; display: grid; gap: 6px; }
  .b-kicker { display: flex; gap: 8px; align-items: center; font-size: 12px; font-weight: 800; letter-spacing: 0.08em; color: #fecaca; }
  .b-title { font-size: clamp(22px, 2.6vw, 34px); font-weight: 850; line-height: 1.1; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .b-sub { font-size: 16px; color: var(--text-2); font-weight: 600; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .b-prog { display: flex; align-items: center; gap: 10px; margin-top: 6px; font-variant-numeric: tabular-nums; }
  .b-prog .bar { flex: 1; max-width: 520px; }
  .b-next { font-size: 14px; color: var(--text-2); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .b-next .dim { font-weight: 800; font-size: 11px; letter-spacing: 0.08em; margin-right: 6px; }
  .b-ctrl { display: grid; grid-template-columns: repeat(5, auto); gap: 8px; flex: none; }

  .slate { position: absolute; inset: 0; display: grid; place-items: center; background: radial-gradient(circle at 50% 40%, #1d2233, #07080c 70%); }
  .scan { position: absolute; inset: 0; background: repeating-linear-gradient(0deg, rgb(255 255 255 / 0.03) 0 2px, transparent 2px 4px); pointer-events: none; }
  .slate-card { position: relative; display: grid; justify-items: center; gap: 10px; text-align: center; padding: 24px; max-width: 520px; }
  .slate-card img { width: 160px; aspect-ratio: 2/3; object-fit: cover; border-radius: 12px; box-shadow: var(--shadow); margin: 8px 0; }
  .slate-kicker { font-family: var(--display); font-size: 22px; letter-spacing: 0.08em; color: var(--accent); margin-top: 8px; }
  .slate-title { font-size: 28px; font-weight: 850; }
  .slate-time { font-family: var(--display); font-size: 18px; color: var(--text-2); }

  .spinner { position: absolute; top: 50%; left: 50%; width: 54px; height: 54px; margin: -27px; border-radius: 50%; border: 4px solid rgb(255 255 255 / 0.15); border-top-color: var(--accent); animation: spin 0.9s linear infinite; }
  @keyframes spin { to { transform: rotate(360deg); } }
  .unmute { position: absolute; top: 50%; left: 50%; transform: translate(-50%, -50%); height: 52px; font-size: 16px; padding: 0 24px; }

  .mg { position: absolute; top: 0; right: 0; bottom: 0; width: min(420px, 100%); display: flex; flex-direction: column; background: rgb(12 14 20 / 0.94); backdrop-filter: blur(18px); border-left: 1px solid var(--line); animation: slide 0.25s ease; }
  @keyframes slide { from { transform: translateX(40px); opacity: 0; } }
  .mg-head { display: flex; align-items: center; justify-content: space-between; padding: 18px 20px; border-bottom: 1px solid var(--line); }
  .mg-list { overflow-y: auto; padding: 8px; display: grid; gap: 4px; }
  .mg-row { display: flex; align-items: center; gap: 12px; padding: 10px; border: 0; border-radius: 12px; background: transparent; text-align: left; }
  .mg-row:hover, .mg-row:focus-visible { background: var(--panel-2); outline-offset: -2px; }
  .mg-row.on { background: rgb(245 165 36 / 0.12); }
  .mg-row img { width: 40px; aspect-ratio: 2/3; object-fit: cover; border-radius: 6px; }
  .mg-info { flex: 1; min-width: 0; display: grid; gap: 3px; }
  .mg-name { font-weight: 750; font-size: 14px; }
  .mg-now { color: var(--text-2); font-size: 13px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }

  @media (max-width: 760px) {
    .banner { left: 10px; right: 10px; bottom: 10px; flex-wrap: wrap; gap: 14px; padding: 14px; }
    .b-left { width: auto; display: flex; }
    .b-ctrl { grid-template-columns: repeat(5, 1fr); width: 100%; }
    .b-ctrl .btn { width: 100%; }
    .clock { margin-right: 80px; font-size: 15px; }
  }
</style>

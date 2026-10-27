<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { api, type MarkerSettings, type ShowCoverage } from '../lib/api';
  import { toast } from '../lib/state.svelte';

  let cfg = $state<MarkerSettings | null>(null);
  let shows = $state<ShowCoverage[]>([]);
  let filter = $state('');
  let poll: ReturnType<typeof setInterval> | undefined;

  async function load() {
    [cfg, shows] = await Promise.all([api.markerSettings(), api.markerShows()]);
    if (cfg.detect.running) watch();
  }

  function watch() {
    clearInterval(poll);
    poll = setInterval(async () => {
      const s = await api.scanStatus();
      if (cfg) cfg.detect = s;
      if (!s.running) {
        clearInterval(poll);
        load();
      }
    }, 2000);
  }

  async function toggle(key: 'server' | 'manual' | 'detect') {
    if (!cfg) return;
    const next = { ...cfg.sources, [key]: !cfg.sources[key] };
    await api.setMarkerSources(next);
    cfg.sources = next;
    toast('Saved — guides on skipping channels updated');
    if (key === 'detect' && next.detect) watch();
  }

  async function scan() {
    if (cfg) cfg.detect = await api.scanMarkers();
    watch();
  }

  onMount(load);
  onDestroy(() => clearInterval(poll));

  const visible = $derived(shows.filter((s) => !filter || s.title.toLowerCase().includes(filter.toLowerCase())));
  const pct = (n: number, d: number) => (d ? Math.round((n / d) * 100) : 0);
</script>

<section class="card pad">
  <h2>Intros &amp; credits</h2>
  <p class="muted small sub">Channels can skip intros and credits (turn it on per channel in the channel editor). Choose where markers come from — when several exist, your own markers win, then Plex/Jellyfin, then auto-detected ones.</p>
  {#if cfg}
    <div class="srcs">
      <label class="src"><input type="checkbox" checked={cfg.sources.server} onchange={() => toggle('server')} />
        <span><b>Plex / Jellyfin markers</b><span class="small muted">Plex Pass detects intros and credits — turn on <i>Generate intro video markers</i> and <i>Generate credits video markers</i> in Plex's library settings. Jellyfin needs a segments plugin such as Intro Skipper.</span></span></label>
      <label class="src"><input type="checkbox" checked={cfg.sources.manual} onchange={() => toggle('manual')} />
        <span><b>My markers</b><span class="small muted">Times you set below, for a whole season or single episodes.</span></span></label>
      <label class="src"><input type="checkbox" checked={cfg.sources.detect} onchange={() => toggle('detect')} />
        <span><b>Auto-detect missing markers</b><span class="small muted">RewindTV compares episodes' audio to find the shared theme song and credits. Only scans shows on channels that skip, one file at a time.{cfg.local_media ? ' Reads files from your mounted media folder.' : ' Reads files through your media server.'}</span></span></label>
    </div>

    {#if cfg.sources.detect || cfg.detect.running || cfg.detect.message}
      <div class="scan">
        {#if cfg.detect.running}
          <div class="row"><span class="spin"></span><span class="small">{cfg.detect.current ?? 'Preparing…'}</span><div class="spacer"></div><span class="small muted">{cfg.detect.done} / {cfg.detect.total}</span></div>
          <div class="bar"><i style:width="{pct(cfg.detect.done, cfg.detect.total)}%"></i></div>
        {:else}
          <div class="row"><span class="small muted">{cfg.detect.message ?? 'No scan yet.'}</span><div class="spacer"></div>
            <button class="btn small" onclick={scan} disabled={!cfg.sources.detect}>Scan now</button></div>
        {/if}
      </div>
    {/if}

    <div class="cov">
      <div><b>{pct(cfg.coverage.intro, cfg.coverage.episodes)}%</b><span>episodes with intro markers</span></div>
      <div><b>{pct(cfg.coverage.credits, cfg.coverage.episodes)}%</b><span>with credits markers</span></div>
      <div class="dir"><b>Saved in</b><code>{cfg.markers_dir}</code><span>One file per show — kept with RewindTV's data, never next to your media, and shared by every server that has the same episodes.</span></div>
    </div>

    <h3 class="shows-h">Shows</h3>
    {#if shows.length > 10}<input type="search" placeholder="Find a show…" bind:value={filter} />{/if}
    <div class="shows">
      {#each visible.slice(0, 200) as s (s.key)}
        <a class="show" href="#/shows/{encodeURIComponent(s.key)}">
          <span class="nm">{s.title}</span>
          <span class="tiny dim">{s.episodes} ep</span>
          <span class="pill" class:full={s.intro === s.episodes} title="Intro markers">Intro {pct(s.intro, s.episodes)}%</span>
          <span class="pill" class:full={s.credits === s.episodes} title="Credits markers">Credits {pct(s.credits, s.episodes)}%</span>
        </a>
      {/each}
    </div>
  {/if}
</section>

<style>
  .pad { padding: 22px; }
  .sub { margin: 6px 0 14px; }
  .srcs { display: grid; gap: 8px; }
  .src { display: flex; gap: 12px; align-items: flex-start; padding: 12px 14px; border-radius: 12px; background: var(--bg-2); border: 1px solid var(--line); cursor: pointer; }
  .src input { margin-top: 3px; width: 18px; height: 18px; accent-color: var(--accent); flex: none; }
  .src > span { display: grid; gap: 3px; }
  .scan { margin-top: 12px; padding: 12px 14px; border-radius: 12px; border: 1px solid var(--line); display: grid; gap: 8px; }
  .spin { width: 14px; height: 14px; border-radius: 50%; border: 2px solid var(--line-2); border-top-color: var(--accent); animation: spin 0.9s linear infinite; }
  @keyframes spin { to { transform: rotate(360deg); } }
  .spacer { flex: 1; }
  .cov { display: grid; grid-template-columns: 1fr 1fr 2fr; gap: 10px; margin-top: 14px; }
  .cov > div { padding: 12px; border-radius: 12px; background: var(--bg-2); border: 1px solid var(--line); display: grid; gap: 2px; }
  .cov b { font-size: 18px; }
  .cov span { color: var(--text-3); font-size: 12px; }
  .cov .dir b { font-size: 13px; }
  .cov code { font-size: 12px; color: var(--text-2); overflow-wrap: anywhere; }
  .shows-h { margin: 18px 0 8px; }
  input[type='search'] { max-width: 300px; height: 36px; margin-bottom: 8px; }
  .shows { display: grid; gap: 4px; max-height: 360px; overflow-y: auto; }
  .show { display: grid; grid-template-columns: 1fr auto auto auto; gap: 10px; align-items: center; padding: 9px 12px; border-radius: 10px; color: var(--text); text-decoration: none; }
  .show:hover, .show:focus-visible { background: var(--panel-2); }
  .nm { font-weight: 650; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .pill { font-size: 11px; font-weight: 750; padding: 3px 8px; border-radius: 999px; background: var(--bg-2); border: 1px solid var(--line-2); color: var(--text-2); }
  .pill.full { color: var(--ok); border-color: rgb(52 211 153 / 0.4); }
  @media (max-width: 640px) { .cov { grid-template-columns: 1fr 1fr; } .cov .dir { grid-column: span 2; } .show { grid-template-columns: 1fr auto; } .show .pill { display: none; } }
</style>

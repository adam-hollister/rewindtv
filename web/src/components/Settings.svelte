<script lang="ts">
  import { onMount } from 'svelte';
  import { api, type Source, type Status, type SyncStatus } from '../lib/api';
  import { session, loadUsers, selectUser, toast } from '../lib/state.svelte';
  import Modal from './Modal.svelte';

  let sources = $state<Source[]>([]);
  let status = $state<Status | null>(null);
  let sync = $state<SyncStatus | null>(null);
  let adding = $state(false);
  let form = $state({ kind: 'jellyfin' as 'plex' | 'jellyfin', url: '', token: '', name: '' });
  let testing = $state<string | null>(null);
  let busy = $state(false);
  let poll: ReturnType<typeof setInterval> | undefined;

  const origin = location.origin;

  async function load() {
    [sources, status, sync] = await Promise.all([api.sources(), api.status(), api.syncStatus()]);
  }

  onMount(() => {
    load();
    return () => clearInterval(poll);
  });

  function watchSync() {
    clearInterval(poll);
    poll = setInterval(async () => {
      sync = await api.syncStatus();
      if (!sync.running) {
        clearInterval(poll);
        load();
        toast(sync.message ?? 'Sync finished');
      }
    }, 1500);
  }

  async function syncNow() {
    sync = await api.syncNow();
    watchSync();
  }

  async function test() {
    testing = 'Testing…';
    const r = await api.testSource(form).catch((e) => ({ ok: false, name: null, error: (e as Error).message }));
    testing = r.ok ? `✓ Connected to ${r.name}` : `✕ ${r.error}`;
  }

  async function add() {
    busy = true;
    try {
      const r = await api.addSource({ ...form, name: form.name || undefined });
      toast(`Added ${r.name}. Syncing your library…`);
      adding = false;
      form = { kind: 'jellyfin', url: '', token: '', name: '' };
      testing = null;
      await load();
      syncNow();
    } catch (e) {
      toast((e as Error).message, 'err');
    } finally {
      busy = false;
    }
  }

  async function removeSource(s: Source) {
    if (!confirm(`Remove ${s.name}? Its items disappear from all channels.`)) return;
    await api.deleteSource(s.id);
    load();
  }

  async function toggleLib(id: number, enabled: boolean) {
    await api.setLibrary(id, enabled);
    load();
  }

  async function rename(id: number, name: string) {
    await api.updateUser(id, { name });
    loadUsers();
  }

  async function removeUser(id: number, name: string) {
    if (!confirm(`Delete profile ${name}? Their private channels are deleted too.`)) return;
    await api.deleteUser(id);
    if (session.userId === id) selectUser(null);
    loadUsers();
  }

  async function rebuild() {
    busy = true;
    await api.rebuildAll().catch((e) => toast((e as Error).message, 'err'));
    busy = false;
    toast('All guides rebuilt');
  }

  function copy(text: string) {
    navigator.clipboard?.writeText(text).then(() => toast('Copied'), () => prompt('Copy this URL', text));
  }

  const ago = (ms: number | null) => {
    if (!ms) return 'never';
    const m = Math.round((Date.now() - ms) / 60000);
    return m < 1 ? 'just now' : m < 60 ? `${m} min ago` : m < 1440 ? `${Math.round(m / 60)} h ago` : `${Math.round(m / 1440)} d ago`;
  };
</script>

<div class="page">
  <h1>Settings</h1>

  <section class="card pad">
    <div class="row wrap"><h2>Media servers</h2><div class="spacer"></div>
      <button class="btn small" onclick={syncNow} disabled={sync?.running || !sources.length}>{sync?.running ? 'Syncing…' : '↻ Sync now'}</button>
      <button class="btn small primary" onclick={() => (adding = true)}>+ Add server</button>
    </div>
    {#if sync?.message}<p class="muted small sub">{sync.running ? '⏳ ' : ''}{sync.message}</p>{/if}
    {#if sources.length === 0}
      <div class="empty"><p class="muted">Connect Plex or Jellyfin to start building channels.</p><button class="btn primary" onclick={() => (adding = true)}>+ Add a media server</button></div>
    {/if}
    <div class="sources">
      {#each sources as s (s.id)}
        <div class="src">
          <div class="row">
            <span class="logo {s.kind}">{s.kind === 'plex' ? 'PLEX' : 'JF'}</span>
            <div class="grow">
              <strong>{s.name}</strong>
              <div class="dim small">{s.url} · synced {ago(s.last_sync)}</div>
              {#if s.last_error}<div class="err small">{s.last_error}</div>{/if}
            </div>
            <button class="btn small ghost danger" onclick={() => removeSource(s)}>Remove</button>
          </div>
          <div class="libs">
            {#each s.libraries as l (l.id)}
              <label class="lib" class:off={!l.enabled}>
                <input type="checkbox" checked={l.enabled} onchange={(e) => toggleLib(l.id, e.currentTarget.checked)} />
                <span>{l.name}</span><span class="dim small">{l.items.toLocaleString()}</span>
              </label>
            {/each}
          </div>
        </div>
      {/each}
    </div>
  </section>

  <section class="card pad">
    <h2>Profiles</h2>
    <p class="muted small sub">Everyone on your network can pick a profile. Each profile has its own favorites, hidden channels and private channels.</p>
    <div class="users">
      {#each session.users as u (u.id)}
        <div class="user">
          <span class="av" style:background={u.color}>{u.name[0]?.toUpperCase()}</span>
          <input type="text" value={u.name} onchange={(e) => rename(u.id, e.currentTarget.value)} aria-label="Profile name" />
          <button class="btn small ghost danger" onclick={() => removeUser(u.id, u.name)}>Delete</button>
        </div>
      {/each}
    </div>
  </section>

  <section class="card pad">
    <h2>Watch in other apps (IPTV)</h2>
    <p class="muted small sub">Add RewindTV as a tuner in <b>Plex Live TV</b>, <b>Jellyfin Live TV</b>, TiviMate, VLC or any IPTV app. Use the M3U playlist plus the XMLTV guide for the profile whose lineup you want.</p>
    <div class="iptv">
      {#each [{ id: 0, name: 'Shared channels only' }, ...session.users] as u}
        {@const m3u = `${origin}/iptv/${u.id}/playlist.m3u`}
        {@const xml = `${origin}/iptv/${u.id}/xmltv.xml`}
        <div class="iptv-row">
          <strong>{u.name}</strong>
          <div class="url"><code>{m3u}</code><button class="btn small" onclick={() => copy(m3u)}>Copy M3U</button></div>
          <div class="url"><code>{xml}</code><button class="btn small" onclick={() => copy(xml)}>Copy XMLTV</button></div>
        </div>
      {/each}
    </div>
  </section>

  <section class="card pad">
    <h2>Maintenance</h2>
    <div class="row wrap maint">
      <button class="btn" onclick={rebuild} disabled={busy}>Rebuild all guides</button>
      <span class="muted small">Regenerates upcoming programs on every channel. What's on now keeps playing.</span>
    </div>
    {#if status}
      <div class="stats">
        <div><b>{status.items.toLocaleString()}</b><span>items</span></div>
        <div><b>{status.channels}</b><span>channels</span></div>
        <div><b>{status.sessions}</b><span>active streams</span></div>
        <div><b>{status.timezone}</b><span>time zone</span></div>
        <div><b>v{status.version}</b><span>RewindTV</span></div>
      </div>
    {/if}
  </section>
</div>

{#if adding}
  <Modal title="Add a media server" onclose={() => (adding = false)}>
    <div class="form">
      <div class="segmented">
        <button class:on={form.kind === 'jellyfin'} onclick={() => (form.kind = 'jellyfin')}>Jellyfin</button>
        <button class:on={form.kind === 'plex'} onclick={() => (form.kind = 'plex')}>Plex</button>
      </div>
      <label class="field">Server URL<input type="url" bind:value={form.url} placeholder={form.kind === 'plex' ? 'http://192.168.1.10:32400' : 'http://192.168.1.10:8096'} /></label>
      <label class="field">{form.kind === 'plex' ? 'Plex token' : 'API key'}<input type="password" bind:value={form.token} autocomplete="off" /></label>
      <p class="help small muted">
        {#if form.kind === 'jellyfin'}
          In Jellyfin: <b>Dashboard → API Keys → +</b>, name it “RewindTV” and paste the key here.
        {:else}
          Open any item in Plex Web → <b>⋯ → Get Info → View XML</b>. The URL ends with <code>X-Plex-Token=…</code> — copy that value.
        {/if}
      </p>
      <label class="field">Display name <span class="dim">(optional)</span><input type="text" bind:value={form.name} /></label>
      {#if testing}<p class="small" class:err={testing.startsWith('✕')} class:ok={testing.startsWith('✓')}>{testing}</p>{/if}
    </div>
    {#snippet footer()}
      <button class="btn" onclick={test} disabled={!form.url || !form.token}>Test</button>
      <button class="btn primary" onclick={add} disabled={!form.url || !form.token || busy}>{busy ? 'Connecting…' : 'Add & sync'}</button>
    {/snippet}
  </Modal>
{/if}

<style>
  .page { display: grid; gap: 20px; max-width: 960px; }
  .pad { padding: 22px; }
  .sub { margin: 6px 0 14px; }
  .spacer { flex: 1; }
  .grow { flex: 1; min-width: 0; }
  .sources { display: grid; gap: 12px; margin-top: 12px; }
  .src { padding: 14px; border-radius: 12px; border: 1px solid var(--line); background: var(--bg-2); display: grid; gap: 12px; }
  .logo { width: 44px; height: 44px; border-radius: 10px; display: grid; place-items: center; font-weight: 900; font-size: 12px; flex: none; }
  .logo.plex { background: #e5a00d; color: #1f1f1f; }
  .logo.jellyfin { background: linear-gradient(135deg, #aa5cc3, #00a4dc); color: #fff; }
  .libs { display: flex; flex-wrap: wrap; gap: 8px; }
  .lib { display: flex; align-items: center; gap: 8px; padding: 6px 12px; border-radius: 999px; border: 1px solid var(--line-2); font-size: 13px; font-weight: 600; cursor: pointer; }
  .lib.off { opacity: 0.5; }
  .lib input { accent-color: var(--accent); }
  .err { color: var(--err); }
  .ok { color: var(--ok); }
  .users { display: grid; gap: 8px; }
  .user { display: flex; align-items: center; gap: 12px; }
  .user input { max-width: 280px; height: 38px; }
  .av { width: 36px; height: 36px; border-radius: 10px; display: grid; place-items: center; font-weight: 850; color: #111; flex: none; }
  .iptv { display: grid; gap: 14px; }
  .iptv-row { display: grid; gap: 6px; padding-bottom: 14px; border-bottom: 1px solid var(--line); }
  .iptv-row:last-child { border-bottom: 0; padding-bottom: 0; }
  .url { display: flex; gap: 8px; align-items: center; }
  code { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 12.5px; padding: 8px 10px; border-radius: 8px; background: var(--bg-2); border: 1px solid var(--line); color: var(--text-2); }
  .help code { padding: 1px 5px; display: inline; white-space: normal; }
  .maint { margin: 12px 0 16px; }
  .stats { display: grid; grid-template-columns: repeat(auto-fit, minmax(130px, 1fr)); gap: 10px; }
  .stats div { padding: 12px; border-radius: 12px; background: var(--bg-2); border: 1px solid var(--line); display: grid; }
  .stats b { font-size: 18px; }
  .stats span { color: var(--text-3); font-size: 12px; }
  .form { display: grid; gap: 14px; }
</style>

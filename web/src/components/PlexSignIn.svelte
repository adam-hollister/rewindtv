<script lang="ts">
  import { onDestroy } from 'svelte';
  import { api, type PlexPin, type PlexServer } from '../lib/api';

  // Calls back with the chosen server's address and token.
  let { onpick }: { onpick: (url: string, token: string, name: string) => void } = $props();

  let stage = $state<'idle' | 'code' | 'servers' | 'error'>('idle');
  let pin = $state<PlexPin | null>(null);
  let servers = $state<PlexServer[]>([]);
  let error = $state('');
  let loadingServers = $state(false);
  let timer: ReturnType<typeof setInterval> | undefined;
  let started = 0;

  async function start() {
    error = '';
    try {
      pin = await api.plexPin();
      stage = 'code';
      started = Date.now();
      clearInterval(timer);
      timer = setInterval(poll, 2000);
    } catch (e) {
      error = (e as Error).message;
      stage = 'error';
    }
  }

  async function poll() {
    if (!pin) return;
    if (Date.now() - started > 10 * 60_000) {
      clearInterval(timer);
      error = 'The code expired. Try again.';
      stage = 'error';
      return;
    }
    try {
      const { token } = await api.plexPinStatus(pin.id);
      if (!token) return;
      clearInterval(timer);
      stage = 'servers';
      loadingServers = true;
      servers = await api.plexServers(token);
      loadingServers = false;
      const owned = servers.filter((s) => s.best);
      if (owned.length === 1) choose(owned[0], owned[0].best!);
    } catch (e) {
      clearInterval(timer);
      error = (e as Error).message;
      stage = 'error';
    }
  }

  function choose(s: PlexServer, uri: string) {
    onpick(uri, s.token, s.name);
  }

  onDestroy(() => clearInterval(timer));
</script>

<div class="psi">
  {#if stage === 'idle' || stage === 'error'}
    <button class="btn plex" onclick={start}>
      <span class="chev">›</span> Sign in with Plex
    </button>
    {#if error}<p class="err small">{error}</p>{/if}
  {:else if stage === 'code' && pin}
    <p class="small muted">On any phone or computer, open <b>plex.tv/link</b> and enter:</p>
    <div class="code">{pin.code}</div>
    <a class="btn small" href={pin.link_url} target="_blank" rel="noopener">Open plex.tv/link ↗</a>
    <p class="tiny dim waiting"><span class="dot"></span> Waiting for you to enter the code…</p>
  {:else if stage === 'servers'}
    {#if loadingServers}
      <p class="small muted"><span class="dot"></span> Signed in. Finding your servers and testing which addresses RewindTV can reach…</p>
    {:else if servers.length === 0}
      <p class="small err">This Plex account has no servers.</p>
    {:else}
      <p class="small muted">Pick the server to use:</p>
      <div class="servers">
        {#each servers as s (s.name + s.token)}
          <div class="srv" class:off={!s.best}>
            <div class="row">
              <strong>{s.name}</strong>
              {#if s.owned}<span class="badge">Yours</span>{/if}
              <div class="spacer"></div>
              {#if s.best}
                <button class="btn small primary" onclick={() => choose(s, s.best!)}>Use this server</button>
              {/if}
            </div>
            <div class="tiny dim">
              {#if s.best}Reachable at {s.best}{:else}Not reachable from RewindTV — is it on the same network?{/if}
            </div>
            {#if !s.best && s.connections.length}
              <details>
                <summary class="tiny">Addresses tried</summary>
                {#each s.connections as c}<div class="tiny dim">{c.local ? 'local' : 'remote'} · {c.uri}</div>{/each}
              </details>
            {/if}
          </div>
        {/each}
      </div>
    {/if}
  {/if}
</div>

<style>
  .psi { display: grid; gap: 10px; justify-items: start; }
  .btn.plex { background: #e5a00d; border-color: #e5a00d; color: #1f1f1f; height: 46px; font-size: 15px; }
  .btn.plex:hover { background: #f3b42b; }
  .chev { font-size: 20px; font-weight: 900; }
  .code { font-family: var(--display); font-size: 44px; letter-spacing: 0.18em; color: var(--accent); padding: 6px 16px; border-radius: 12px; background: var(--bg-2); border: 1px solid var(--line-2); }
  .waiting { display: flex; align-items: center; gap: 8px; }
  .dot { display: inline-block; width: 8px; height: 8px; border-radius: 50%; background: var(--accent); animation: blink 1.2s infinite; }
  @keyframes blink { 50% { opacity: 0.2; } }
  .servers { display: grid; gap: 8px; width: 100%; }
  .srv { padding: 12px; border-radius: 12px; border: 1px solid var(--line); background: var(--bg-2); display: grid; gap: 4px; }
  .srv.off { opacity: 0.7; }
  .spacer { flex: 1; }
  .err { color: var(--err); }
</style>

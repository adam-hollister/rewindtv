<script lang="ts">
  import { onMount } from 'svelte';
  import { api, type ChannelListItem } from '../lib/api';
  import type { RemoteCommand } from '../lib/remote.svelte';
  import { clockState } from '../lib/state.svelte';
  import { progress } from '../lib/time';
  import ChannelBadge from './ChannelBadge.svelte';

  const KEY = 'rewindtv.remote-code';
  let code = $state(load());
  let entry = $state('');
  let paired = $state(false);
  let checking = $state(false);
  let error = $state<string | null>(null);
  let digits = $state('');
  let lineup = $state<ChannelListItem[]>([]);
  let tab = $state<'pad' | 'list'>('pad');

  function load(): string {
    try { return localStorage.getItem(KEY) ?? ''; } catch { return ''; }
  }

  async function pair(c: string) {
    checking = true;
    error = null;
    try {
      const s = await api.remoteStatus(c);
      if (s.connected) {
        code = c;
        paired = true;
        try { localStorage.setItem(KEY, c); } catch { /* ignore */ }
        lineup = (await api.channels()).filter((x) => !x.hidden);
      } else {
        paired = false;
        error = 'No TV is showing that code. Open a channel on the TV and press R to see it.';
      }
    } catch (e) {
      error = (e as Error).message;
    } finally {
      checking = false;
    }
  }

  async function send(c: RemoteCommand) {
    navigator.vibrate?.(15);
    try {
      await api.remoteSend(code, c);
      error = null;
    } catch {
      paired = false;
      error = 'Lost the TV. Check the code on screen (press R on the TV).';
    }
  }

  function press(d: string) {
    digits = (digits + d).slice(-4);
  }
  function enter() {
    if (digits) send({ cmd: 'number', number: Number(digits) });
    digits = '';
  }

  onMount(() => {
    if (code) pair(code);
    const t = setInterval(async () => {
      if (paired) lineup = (await api.channels().catch(() => lineup)).filter((x) => !x.hidden);
    }, 60_000);
    return () => clearInterval(t);
  });
</script>

<main class="remote">
  <header><span class="logo">REWIND<b>TV</b></span><span class="dim">Remote</span></header>

  {#if !paired}
    <section class="pairing">
      <h1>Pair with your TV</h1>
      <p class="muted">On the TV, open any channel and press <b>R</b> (or tap 📱 at the top). Enter the 4-digit code it shows.</p>
      <form onsubmit={(e) => { e.preventDefault(); pair(entry); }}>
        <input class="code" inputmode="numeric" pattern={'[0-9]{4}'} maxlength="4" placeholder="0000" bind:value={entry} autocomplete="off" />
        <button class="btn primary" disabled={entry.length !== 4 || checking}>{checking ? 'Connecting…' : 'Connect'}</button>
      </form>
      {#if error}<p class="err">{error}</p>{/if}
    </section>
  {:else}
    <div class="status"><span class="dot"></span> TV {code} <button class="btn small ghost" onclick={() => { paired = false; entry = ''; }}>Change</button></div>
    {#if error}<p class="err">{error}</p>{/if}

    <div class="segmented tabs">
      <button class:on={tab === 'pad'} onclick={() => (tab = 'pad')}>Remote</button>
      <button class:on={tab === 'list'} onclick={() => (tab = 'list')}>What's on</button>
    </div>

    {#if tab === 'pad'}
      <section class="pad">
        <div class="rocker">
          <button class="big" onclick={() => send({ cmd: 'up' })} aria-label="Channel up">▲</button>
          <span class="dim">CH</span>
          <button class="big" onclick={() => send({ cmd: 'down' })} aria-label="Channel down">▼</button>
        </div>
        <div class="side">
          <button onclick={() => send({ cmd: 'info' })}>ℹ︎ Info</button>
          <button onclick={() => send({ cmd: 'guide' })}>☰ List</button>
          <button onclick={() => send({ cmd: 'last' })}>↩ Last</button>
          <button onclick={() => send({ cmd: 'start_over' })}>⏮ Start over</button>
          <button onclick={() => send({ cmd: 'mute' })}>🔇 Mute</button>
        </div>
      </section>
      <div class="entry">{digits || ' '}</div>
      <section class="numpad">
        {#each ['1', '2', '3', '4', '5', '6', '7', '8', '9'] as d}
          <button onclick={() => press(d)}>{d}</button>
        {/each}
        <button class="dim" onclick={() => (digits = '')}>Clear</button>
        <button onclick={() => press('0')}>0</button>
        <button class="go" onclick={enter}>Go</button>
      </section>
    {:else}
      <section class="list">
        {#each lineup as c (c.id)}
          <button class="row" onclick={() => send({ cmd: 'tune', channel_id: c.id })}>
            <ChannelBadge number={c.number} color={c.color} id={c.id} logo={c.logo} />
            <span class="info">
              <span class="name">{c.name}</span>
              <span class="now">{c.now?.title ?? '—'}{c.now?.subtitle ? ` · ${c.now.subtitle}` : ''}</span>
              {#if c.now}<span class="bar"><i style:width="{progress(c.now, clockState.now) * 100}%"></i></span>{/if}
            </span>
          </button>
        {/each}
      </section>
    {/if}
  {/if}
</main>

<style>
  .remote { max-width: 440px; margin: 0 auto; padding: 16px 16px 40px; display: grid; gap: 16px; }
  header { display: flex; align-items: baseline; gap: 10px; }
  .logo { font-family: var(--display); font-size: 22px; letter-spacing: 0.04em; }
  .logo b { color: var(--accent); font-weight: inherit; }
  h1 { font-size: 24px; margin: 8px 0 4px; }
  .pairing form { display: grid; gap: 12px; margin-top: 12px; }
  .code { font-family: var(--display); font-size: 48px; letter-spacing: 0.3em; text-align: center; padding: 12px; border-radius: 16px; background: var(--panel); border: 1px solid var(--line-2); color: inherit; width: 100%; box-sizing: border-box; }
  .err { color: #fca5a5; margin: 0; }
  .status { display: flex; align-items: center; gap: 8px; font-weight: 600; }
  .status .btn { margin-left: auto; }
  .dot { width: 10px; height: 10px; border-radius: 50%; background: #22c55e; box-shadow: 0 0 10px #22c55e; }
  .tabs { display: flex; }
  .tabs button { flex: 1; }
  button { font: inherit; color: inherit; cursor: pointer; -webkit-tap-highlight-color: transparent; }
  .pad { display: grid; grid-template-columns: 1fr 1.3fr; gap: 12px; }
  .rocker { display: grid; grid-template-rows: 1fr auto 1fr; gap: 6px; text-align: center; background: var(--panel); border: 1px solid var(--line-2); border-radius: 40px; padding: 8px; }
  .rocker .big { font-size: 34px; border: none; background: var(--panel-2); border-radius: 32px; min-height: 96px; }
  .side { display: grid; gap: 8px; }
  .side button, .numpad button { border: 1px solid var(--line-2); background: var(--panel); border-radius: 14px; padding: 12px; font-weight: 600; min-height: 48px; }
  .pad button:active, .numpad button:active, .row:active { transform: scale(0.97); background: var(--panel-2); }
  .entry { text-align: center; font-family: var(--display); font-size: 36px; letter-spacing: 0.2em; color: var(--accent); min-height: 44px; }
  .numpad { display: grid; grid-template-columns: repeat(3, 1fr); gap: 10px; }
  .numpad button { font-size: 24px; min-height: 60px; }
  .numpad .dim { font-size: 15px; }
  .numpad .go { background: var(--accent); color: #111; border-color: transparent; font-size: 18px; }
  .list { display: grid; gap: 8px; }
  .row { display: flex; align-items: center; gap: 12px; text-align: left; padding: 10px 12px; border: 1px solid var(--line); border-radius: 14px; background: var(--panel); }
  .info { display: grid; gap: 3px; min-width: 0; flex: 1; }
  .name { font-weight: 700; }
  .now { color: var(--muted); font-size: 14px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .bar { height: 3px; background: var(--line-2); border-radius: 2px; overflow: hidden; }
  .bar i { display: block; height: 100%; background: var(--accent); }
</style>

<script lang="ts">
  import { currentUser, go, clockState } from '../lib/state.svelte';
  import { clock } from '../lib/time';

  let { page }: { page: string } = $props();
  const user = $derived(currentUser());
  const tabs = [
    ['guide', 'Guide'],
    ['channels', 'Channels'],
    ['settings', 'Settings'],
  ];
</script>

<header>
  <a class="brand" href="#/guide" aria-label="RewindTV home">
    <svg viewBox="0 0 64 64" width="30" height="30" aria-hidden="true"><rect x="4" y="12" width="56" height="40" rx="10" fill="var(--accent)"/><rect x="10" y="18" width="44" height="28" rx="6" fill="#0a0c11"/><path d="M27 25l12 7-12 7z" fill="var(--accent)"/><path d="M22 6l10 6 10-6" stroke="var(--accent)" stroke-width="4" fill="none" stroke-linecap="round"/></svg>
    <span>REWIND<b>TV</b></span>
  </a>
  <nav>
    {#each tabs as [id, label]}
      <a href="#/{id}" class:on={page === id}>{label}</a>
    {/each}
  </nav>
  <div class="spacer"></div>
  <div class="clock">{clock(clockState.now)}</div>
  {#if user}
    <button class="me" onclick={() => go('profiles')} title="Switch profile">
      <span class="av" style:background={user.color}>{user.name[0]?.toUpperCase()}</span>
      <span class="nm">{user.name}</span>
    </button>
  {/if}
</header>

<style>
  header {
    position: sticky; top: 0; z-index: 20;
    display: flex; align-items: center; gap: 20px;
    height: 64px; padding: 0 24px;
    background: rgb(10 12 17 / 0.82); backdrop-filter: blur(14px) saturate(1.4);
    border-bottom: 1px solid var(--line);
  }
  .brand { display: flex; align-items: center; gap: 10px; text-decoration: none; color: var(--text); }
  .brand span { font-family: var(--display); font-size: 20px; letter-spacing: 0.04em; }
  .brand b { color: var(--accent); font-weight: inherit; }
  nav { display: flex; gap: 4px; }
  nav a { padding: 8px 14px; border-radius: 999px; color: var(--text-2); text-decoration: none; font-weight: 650; font-size: 14px; }
  nav a:hover { color: var(--text); background: var(--panel); }
  nav a.on { color: var(--text); background: var(--panel-2); box-shadow: inset 0 0 0 1px var(--line-2); }
  .spacer { flex: 1; }
  .clock { font-variant-numeric: tabular-nums; font-weight: 700; color: var(--text-2); }
  .me { display: flex; align-items: center; gap: 8px; border: 0; background: none; padding: 4px; border-radius: 999px; }
  .av { width: 34px; height: 34px; border-radius: 50%; display: grid; place-items: center; font-weight: 800; color: #111; }
  .nm { font-weight: 650; font-size: 14px; }
  @media (max-width: 720px) {
    header { gap: 10px; padding: 0 12px; }
    .brand span, .clock, .nm { display: none; }
    nav a { padding: 8px 10px; }
  }
</style>

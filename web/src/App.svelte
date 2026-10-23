<script lang="ts">
  import { onMount } from 'svelte';
  import { session, route, loadUsers, toasts } from './lib/state.svelte';
  import { installSpatialNav } from './lib/spatial';
  import ProfilePicker from './components/ProfilePicker.svelte';
  import TopBar from './components/TopBar.svelte';
  import Guide from './components/Guide.svelte';
  import Watch from './components/Watch.svelte';
  import Channels from './components/Channels.svelte';
  import ChannelEditor from './components/ChannelEditor.svelte';
  import Settings from './components/Settings.svelte';

  onMount(() => {
    installSpatialNav();
    loadUsers();
  });

  const page = $derived(route.parts[0] ?? 'guide');
</script>

{#if !session.loaded}
  <div class="boot"><span class="logo">REWIND<b>TV</b></span></div>
{:else if session.userId === null || page === 'profiles'}
  <ProfilePicker />
{:else if page === 'watch'}
  {#key route.parts[1]}
    <Watch channelId={Number(route.parts[1])} />
  {/key}
{:else}
  <TopBar {page} />
  {#if page === 'channels' && route.parts[1]}
    {#key route.parts[1]}
      <ChannelEditor id={route.parts[1] === 'new' ? null : Number(route.parts[1])} />
    {/key}
  {:else if page === 'channels'}
    <Channels />
  {:else if page === 'settings'}
    <Settings />
  {:else}
    <Guide />
  {/if}
{/if}

<div class="toasts" aria-live="polite">
  {#each toasts.list as t (t.id)}
    <div class="toast {t.kind}">{t.text}</div>
  {/each}
</div>

<style>
  .boot { height: 100vh; display: grid; place-items: center; }
  .logo { font-family: var(--display); font-size: 40px; letter-spacing: 0.04em; opacity: 0.8; }
  .logo b { color: var(--accent); font-weight: inherit; }
  .toasts { position: fixed; bottom: 20px; left: 50%; transform: translateX(-50%); display: grid; gap: 8px; z-index: 100; }
  .toast { padding: 12px 18px; border-radius: 12px; background: var(--panel-2); border: 1px solid var(--line-2); box-shadow: var(--shadow); font-weight: 600; animation: rise 0.25s ease; }
  .toast.err { border-color: rgb(248 113 113 / 0.6); color: #fecaca; }
  @keyframes rise { from { transform: translateY(10px); opacity: 0; } }
</style>

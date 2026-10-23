<script lang="ts">
  import { onMount } from 'svelte';
  import { api, type Suggestion } from '../lib/api';
  import { session, toast } from '../lib/state.svelte';
  import Modal from './Modal.svelte';

  let { onclose, ondone }: { onclose: () => void; ondone: () => void } = $props();

  const KINDS: [string, string][] = [
    ['genre', 'Genres'],
    ['decade', 'Decades'],
    ['show', 'Show 24/7'],
    ['collection', 'Collections'],
    ['studio', 'Networks & studios'],
  ];
  let all = $state<Suggestion[]>([]);
  let loaded = $state(false);
  let kind = $state('genre');
  let picked = $state(new Set<string>());
  let shared = $state(true);
  let busy = $state(false);

  const key = (s: Suggestion) => `${s.kind}:${s.name}`;
  const list = $derived(all.filter((s) => s.kind === kind));

  onMount(async () => {
    all = await api.suggestions();
    loaded = true;
    const firstKind = KINDS.find(([k]) => all.some((s) => s.kind === k));
    if (firstKind) kind = firstKind[0];
  });

  function toggle(s: Suggestion) {
    const k = key(s);
    const next = new Set(picked);
    next.has(k) ? next.delete(k) : next.add(k);
    picked = next;
  }

  async function create() {
    busy = true;
    try {
      const chosen = all.filter((s) => picked.has(key(s))).map((s) => ({ name: s.name, rule: s.rule, ordering: s.ordering }));
      const ids = await api.autogen(chosen, shared ? null : session.userId);
      toast(`Created ${ids.length} channel${ids.length === 1 ? '' : 's'}`);
      ondone();
    } catch (e) {
      toast((e as Error).message, 'err');
    } finally {
      busy = false;
    }
  }
</script>

<Modal title="Auto-create channels" {onclose} wide>
  {#if !loaded}
    <p class="muted">Looking through your library…</p>
  {:else if all.length === 0}
    <div class="empty"><p class="muted">No suggestions yet. Connect a media server in Settings and run a sync first.</p></div>
  {:else}
    <p class="muted small">Pick as many as you like. Each becomes a channel you can fine-tune later.</p>
    <div class="tabs scroll-x">
      {#each KINDS as [k, label]}
        {@const n = all.filter((s) => s.kind === k).length}
        {#if n}
          <button class="chip" class:on={kind === k} onclick={() => (kind = k)}>{label} <span class="dim">{n}</span></button>
        {/if}
      {/each}
    </div>
    <div class="list">
      {#each list as s (key(s))}
        <label class="opt" class:on={picked.has(key(s))}>
          <input type="checkbox" checked={picked.has(key(s))} onchange={() => toggle(s)} />
          <span class="nm">{s.name}</span>
          <span class="dim small">{s.items} items · {Math.round(s.hours)}h</span>
          <span class="badge">{s.ordering}</span>
        </label>
      {/each}
    </div>
  {/if}
  {#snippet footer()}
    <div class="segmented">
      <button class:on={shared} onclick={() => (shared = true)}>Shared</button>
      <button class:on={!shared} onclick={() => (shared = false)}>Only me</button>
    </div>
    <div class="spacer"></div>
    <button class="btn ghost" onclick={onclose}>Cancel</button>
    <button class="btn primary" disabled={!picked.size || busy} onclick={create}>Create {picked.size || ''} channel{picked.size === 1 ? '' : 's'}</button>
  {/snippet}
</Modal>

<style>
  .tabs { display: flex; gap: 8px; margin: 14px 0; padding-bottom: 4px; }
  .list { display: grid; grid-template-columns: repeat(auto-fill, minmax(250px, 1fr)); gap: 8px; }
  .opt { display: grid; grid-template-columns: auto 1fr auto; grid-template-rows: auto auto; column-gap: 10px; align-items: center; padding: 12px; border-radius: 12px; border: 1px solid var(--line); background: var(--bg-2); cursor: pointer; }
  .opt.on { border-color: rgb(245 165 36 / 0.6); background: rgb(245 165 36 / 0.08); }
  .opt input { grid-row: span 2; width: 18px; height: 18px; accent-color: var(--accent); }
  .nm { font-weight: 700; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .opt .badge { grid-row: span 2; grid-column: 3; grid-row-start: 1; }
  .spacer { flex: 1; }
</style>

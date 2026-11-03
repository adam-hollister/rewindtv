<script lang="ts">
  import { api, type SearchHit } from '../lib/api';
  import { session, go, hasReminder, toggleReminder, reminders, toast } from '../lib/state.svelte';
  import { clock } from '../lib/time';
  import ChannelBadge from './ChannelBadge.svelte';

  // "When is Seinfeld on?" — searches the week ahead in your lineup.
  let q = $state('');
  let hits = $state<SearchHit[] | null>(null);
  let open = $state(false);
  let timer: ReturnType<typeof setTimeout> | undefined;

  function onInput() {
    clearTimeout(timer);
    if (q.trim().length < 2) {
      hits = null;
      return;
    }
    timer = setTimeout(async () => {
      hits = await api.search(q.trim(), session.userId ?? undefined).catch(() => []);
      open = true;
    }, 250);
  }

  // Group airings by title, soonest first, a few per title.
  const groups = $derived.by(() => {
    const m = new Map<string, SearchHit[]>();
    for (const h of hits ?? []) {
      const list = m.get(h.title) ?? [];
      if (list.length < 6) list.push(h);
      m.set(h.title, list);
    }
    return [...m.entries()];
  });

  const day = (ms: number) => {
    const d = new Date(ms);
    const today = new Date();
    const tomorrow = new Date(Date.now() + 86_400_000);
    if (d.toDateString() === today.toDateString()) return 'Today';
    if (d.toDateString() === tomorrow.toDateString()) return 'Tomorrow';
    return d.toLocaleDateString(undefined, { weekday: 'short' });
  };

  async function remind(h: SearchHit) {
    try {
      const on = hasReminder(h.program_id);
      await toggleReminder(h.program_id);
      toast(on ? 'Reminder removed' : `We'll remind you before ${h.title} starts`);
    } catch (e) {
      toast((e as Error).message, 'err');
    }
  }
</script>

<div class="search">
  <input type="search" placeholder="Search the guide…" bind:value={q} oninput={onInput} onfocus={() => (open = !!hits)} aria-label="Search the guide" />
  {#if open && hits}
    <div class="panel" role="listbox">
      <div class="row head"><span class="small muted">{hits.length ? `Next 7 days · ${hits.length} airings` : `Nothing coming up for “${q}”`}</span><div class="spacer"></div><button class="btn small ghost" onclick={() => (open = false)}>Close</button></div>
      {#each groups as [title, list] (title)}
        <div class="group">
          <div class="gt">{#if list[0].thumb}<img src={list[0].thumb} alt="" />{/if}<b>{title}</b></div>
          {#each list as h (h.program_id)}
            {@const live = h.start <= Date.now() && h.end > Date.now()}
            <div class="hit">
              <span class="when">{live ? 'On now' : `${day(h.start)} ${clock(h.start)}`}</span>
              <ChannelBadge number={h.channel_number} color={h.channel_color} id={h.channel_id} logo={h.channel_logo} />
              <span class="sub small muted">{h.subtitle ?? h.channel_name}</span>
              {#if live}
                <button class="btn small primary" onclick={() => go(`watch/${h.channel_id}`)}>Watch</button>
              {:else}
                {#key reminders.list}
                  <button class="btn small" class:on={hasReminder(h.program_id)} onclick={() => remind(h)} title="Remind me">{hasReminder(h.program_id) ? '🔔 Set' : '🔔 Remind'}</button>
                {/key}
              {/if}
            </div>
          {/each}
        </div>
      {/each}
    </div>
  {/if}
</div>

<style>
  .search { position: relative; }
  input { width: 260px; height: 36px; border-radius: 999px; padding: 0 14px; }
  .panel {
    position: absolute; top: 44px; left: 0; z-index: 30; width: min(560px, calc(100vw - 32px)); max-height: 60vh; overflow-y: auto;
    background: var(--panel); border: 1px solid var(--line-2); border-radius: 14px; box-shadow: var(--shadow); padding: 8px;
  }
  .head { padding: 4px 6px 8px; }
  .spacer { flex: 1; }
  .group { padding: 8px 6px; border-top: 1px solid var(--line); }
  .gt { display: flex; align-items: center; gap: 10px; margin-bottom: 6px; }
  .gt img { width: 28px; aspect-ratio: 2/3; object-fit: cover; border-radius: 4px; }
  .hit { display: grid; grid-template-columns: 110px auto 1fr auto; gap: 10px; align-items: center; padding: 4px 0; }
  .when { font-weight: 650; font-size: 13px; font-variant-numeric: tabular-nums; }
  .sub { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .btn.on { border-color: var(--accent); color: var(--accent-2); }
  @media (max-width: 700px) { input { width: 160px; } .hit { grid-template-columns: 90px auto 1fr auto; } }
</style>

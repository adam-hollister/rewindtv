<script lang="ts">
  import { onMount } from 'svelte';
  import { api, emptyRule, type Channel, type Facets, type Ordering, type PreviewProgram, type Slot } from '../lib/api';
  import { session, go, toast } from '../lib/state.svelte';
  import { clock, DAYS, minToTime, timeToMin, daysLabel, duration } from '../lib/time';
  import RuleEditor from './RuleEditor.svelte';
  import WeekTimeline from './WeekTimeline.svelte';
  import ChannelBadge from './ChannelBadge.svelte';

  let { id }: { id: number | null } = $props();

  const COLORS = ['#f59e0b', '#ef4444', '#10b981', '#3b82f6', '#8b5cf6', '#ec4899', '#14b8a6', '#f97316', '#84cc16', '#06b6d4'];
  const ORDERINGS: { v: Ordering; title: string; desc: string }[] = [
    { v: 'shuffle', title: 'Shuffle', desc: 'Random mix. Everything airs before anything repeats.' },
    { v: 'episodic', title: 'Rotation', desc: 'One episode per show, in order, cycling between shows.' },
    { v: 'block', title: 'Blocks', desc: 'A few episodes of one show back-to-back, then the next show.' },
    { v: 'marathon', title: 'Marathon', desc: 'Whole series start to finish, then the next one.' },
  ];

  let ch = $state<Channel>({
    name: '', number: null, color: COLORS[Math.floor(Math.random() * COLORS.length)], owner_user_id: null,
    rule: emptyRule(), ordering: 'shuffle', block_size: 3, avoid_repeat_hours: 24, align_minutes: 0, slots: [],
  });
  let facets = $state<Facets | null>(null);
  let openSlot = $state<number | null>(null);
  let preview = $state<PreviewProgram[] | null>(null);
  let previewing = $state(false);
  let saving = $state(false);

  onMount(async () => {
    const [f, c] = await Promise.all([api.facets(), id !== null ? api.channel(id) : Promise.resolve(null)]);
    facets = f;
    if (c) {
      c.rule = { ...emptyRule(), ...c.rule };
      c.slots.forEach((s) => (s.rule = { ...emptyRule(), ...s.rule }));
      ch = c;
    }
  });

  function addSlot() {
    ch.slots.push({ title: `Block ${ch.slots.length + 1}`, days: 127, start_min: 18 * 60, end_min: 22 * 60, rule: emptyRule(), ordering: 'episodic' });
    openSlot = ch.slots.length - 1;
  }

  function addPreset(title: string, days: number, start: number, end: number, ordering: Ordering) {
    ch.slots.push({ title, days, start_min: start, end_min: end, rule: emptyRule(), ordering });
    openSlot = ch.slots.length - 1;
  }

  function toggleDay(s: Slot, d: number) {
    s.days ^= 1 << d;
  }

  async function runPreview() {
    previewing = true;
    try {
      preview = await api.previewChannel($state.snapshot(ch) as Channel);
    } catch (e) {
      toast((e as Error).message, 'err');
    } finally {
      previewing = false;
    }
  }

  async function save() {
    saving = true;
    try {
      const body = $state.snapshot(ch) as Channel;
      if (id === null) await api.createChannel(body);
      else await api.updateChannel(id, body);
      toast(id === null ? `Created ${ch.name}` : `Saved — guide rebuilt for ${ch.name}`);
      go('channels');
    } catch (e) {
      toast((e as Error).message, 'err');
    } finally {
      saving = false;
    }
  }

  async function remove() {
    if (id === null || !confirm(`Delete “${ch.name}”?`)) return;
    await api.deleteChannel(id);
    go('channels');
  }
</script>

<div class="page">
  <div class="head">
    <a class="btn ghost small" href="#/channels">← Channels</a>
    <h1>{id === null ? 'New channel' : `Edit ${ch.name || 'channel'}`}</h1>
  </div>

  {#if !facets}
    <p class="muted">Loading…</p>
  {:else}
    <div class="layout">
      <div class="main">
        <section class="card pad">
          <div class="ident">
            <ChannelBadge number={ch.number ?? 0} color={ch.color ?? COLORS[0]} size="xl" />
            <div class="ident-fields">
              <div class="two">
                <label class="field">Channel name<input type="text" bind:value={ch.name} placeholder="Saturday Morning Toons" /></label>
                <label class="field num">Number<input type="number" bind:value={ch.number} placeholder="auto" min="1" /></label>
              </div>
              <div class="row wrap">
                <div class="colors">
                  {#each COLORS as c}
                    <button type="button" class="sw" class:on={ch.color === c} style:background={c} onclick={() => (ch.color = c)} aria-label="Color {c}"></button>
                  {/each}
                </div>
                <div class="spacer"></div>
                <div class="segmented">
                  <button type="button" class:on={ch.owner_user_id === null} onclick={() => (ch.owner_user_id = null)}>Shared</button>
                  <button type="button" class:on={ch.owner_user_id !== null} onclick={() => (ch.owner_user_id = session.userId)}>Only me</button>
                </div>
              </div>
            </div>
          </div>
        </section>

        <section class="card pad">
          <h2>What plays</h2>
          <p class="muted small sub">Leave everything empty to play the whole library.</p>
          <RuleEditor bind:rule={ch.rule} {facets} />
        </section>

        <section class="card pad">
          <h2>How it plays</h2>
          <div class="orders">
            {#each ORDERINGS as o}
              <button type="button" class="order" class:on={ch.ordering === o.v} onclick={() => (ch.ordering = o.v)}>
                <strong>{o.title}</strong><span class="small muted">{o.desc}</span>
              </button>
            {/each}
          </div>
          <div class="opts">
            {#if ch.ordering === 'block'}
              <label class="field">Episodes per block<input type="number" min="1" max="20" bind:value={ch.block_size} /></label>
            {/if}
            <label class="field">Don't repeat within (hours)<input type="number" min="0" max="720" bind:value={ch.avoid_repeat_hours} /></label>
            <div class="field">
              Start shows on
              <div class="segmented">
                {#each [[0, 'Any minute'], [15, ':15'], [30, ':00 / :30']] as [v, l]}
                  <button type="button" class:on={ch.align_minutes === v} onclick={() => (ch.align_minutes = v as number)}>{l}</button>
                {/each}
              </div>
            </div>
          </div>
        </section>

        <section class="card pad">
          <div class="row"><h2>Time-of-day blocks</h2><div class="spacer"></div><button type="button" class="btn small" onclick={addSlot}>+ Add block</button></div>
          <p class="muted small sub">Program the channel like a real network: cartoons on Saturday mornings, sitcoms at prime time, movies late at night. Outside any block, the channel plays its main lineup above.</p>
          {#if ch.slots.length === 0}
            <div class="presets">
              <button type="button" class="chip" onclick={() => addPreset('Saturday Morning Cartoons', 1 << 5, 7 * 60, 12 * 60, 'episodic')}>🥣 Saturday morning cartoons</button>
              <button type="button" class="chip" onclick={() => addPreset('Prime Time', 127, 20 * 60, 23 * 60, 'episodic')}>🌙 Prime time</button>
              <button type="button" class="chip" onclick={() => addPreset('After School', 31, 15 * 60, 18 * 60, 'block')}>🎒 After school</button>
              <button type="button" class="chip" onclick={() => addPreset('Late Night Movies', 127, 23 * 60, 3 * 60, 'shuffle')}>🎬 Late-night movies</button>
            </div>
          {:else}
            <WeekTimeline slots={ch.slots} colors={COLORS} onpick={(i) => (openSlot = i)} />
            <div class="slots">
              {#each ch.slots as s, i}
                <div class="slot" class:open={openSlot === i} style:--c={COLORS[i % COLORS.length]}>
                  <button type="button" class="slot-head" onclick={() => (openSlot = openSlot === i ? null : i)}>
                    <i class="dot"></i>
                    <strong>{s.title}</strong>
                    <span class="muted small">{daysLabel(s.days)} · {minToTime(s.start_min)}–{minToTime(s.end_min)}</span>
                    <span class="spacer"></span>
                    <span class="dim">{openSlot === i ? '▲' : '▼'}</span>
                  </button>
                  {#if openSlot === i}
                    <div class="slot-body">
                      <div class="two">
                        <label class="field">Block name<input type="text" bind:value={s.title} /></label>
                        <label class="field">Starts<input type="time" value={minToTime(s.start_min)} onchange={(e) => (s.start_min = timeToMin(e.currentTarget.value))} /></label>
                        <label class="field">Ends<input type="time" value={minToTime(s.end_min)} onchange={(e) => (s.end_min = timeToMin(e.currentTarget.value))} /></label>
                      </div>
                      <div class="days">
                        {#each DAYS as d, di}
                          <button type="button" class="chip" class:on={s.days & (1 << di)} onclick={() => toggleDay(s, di)}>{d}</button>
                        {/each}
                      </div>
                      <div class="segmented">
                        {#each ORDERINGS as o}<button type="button" class:on={s.ordering === o.v} onclick={() => (s.ordering = o.v)}>{o.title}</button>{/each}
                      </div>
                      <RuleEditor bind:rule={s.rule} {facets} compact />
                      <button type="button" class="btn small ghost danger" onclick={() => { ch.slots.splice(i, 1); openSlot = null; }}>Remove block</button>
                    </div>
                  {/if}
                </div>
              {/each}
            </div>
          {/if}
        </section>
      </div>

      <aside>
        <div class="card pad sticky">
          <div class="row"><h3>Next 24 hours</h3><div class="spacer"></div><button type="button" class="btn small" onclick={runPreview} disabled={previewing}>{preview ? 'Refresh' : 'Preview'}</button></div>
          {#if preview}
            <ol class="pv">
              {#each preview.slice(0, 80) as p}
                <li class:filler={p.filler}>
                  <span class="tm">{clock(p.start)}</span>
                  <span class="pt"><b>{p.title}</b>{#if p.subtitle}<span class="dim small psub">{p.subtitle}</span>{/if}
                    {#if p.block}<span class="badge">{p.block}</span>{/if}</span>
                  <span class="dim tiny">{duration(p.end - p.start)}</span>
                </li>
              {/each}
            </ol>
          {:else}
            <p class="muted small">See exactly what this channel will air before you save.</p>
          {/if}
          <div class="save">
            {#if id !== null}<button type="button" class="btn ghost danger" onclick={remove}>Delete</button>{/if}
            <div class="spacer"></div>
            <a class="btn ghost" href="#/channels">Cancel</a>
            <button type="button" class="btn primary" onclick={save} disabled={saving || !ch.name.trim()}>{id === null ? 'Create channel' : 'Save'}</button>
          </div>
        </div>
      </aside>
    </div>
  {/if}
</div>

<style>
  .head { display: flex; align-items: center; gap: 12px; margin-bottom: 20px; }
  .layout { display: grid; grid-template-columns: minmax(0, 1fr) 360px; gap: 20px; align-items: start; }
  .main { display: grid; gap: 20px; }
  .pad { padding: 22px; }
  .sub { margin: 6px 0 16px; }
  .ident { display: flex; gap: 20px; align-items: center; }
  .ident-fields { flex: 1; display: grid; gap: 14px; }
  .two { display: grid; grid-template-columns: 1fr 120px; gap: 12px; }
  .slot-body .two { grid-template-columns: 1fr 130px 130px; }
  .colors { display: flex; gap: 8px; flex-wrap: wrap; }
  .sw { width: 26px; height: 26px; border-radius: 50%; border: 0; }
  .sw.on { box-shadow: 0 0 0 3px var(--bg), 0 0 0 5px var(--text); }
  .orders { display: grid; grid-template-columns: repeat(auto-fit, minmax(170px, 1fr)); gap: 10px; margin-top: 14px; }
  .order { display: grid; gap: 4px; text-align: left; padding: 14px; border-radius: 12px; border: 1px solid var(--line-2); background: var(--bg-2); }
  .order.on { border-color: var(--accent); background: rgb(245 165 36 / 0.08); box-shadow: inset 0 0 0 1px var(--accent); }
  .opts { display: flex; gap: 16px; flex-wrap: wrap; margin-top: 16px; align-items: end; }
  .opts .field { width: auto; }
  .opts input { width: 160px; }
  .presets, .days { display: flex; gap: 8px; flex-wrap: wrap; }
  .slots { display: grid; gap: 8px; margin-top: 16px; }
  .slot { border: 1px solid var(--line); border-radius: 12px; background: var(--bg-2); }
  .slot.open { border-color: color-mix(in srgb, var(--c) 50%, var(--line)); }
  .slot-head { width: 100%; display: flex; align-items: center; gap: 10px; padding: 12px 14px; border: 0; background: none; text-align: left; flex-wrap: wrap; }
  .dot { width: 10px; height: 10px; border-radius: 3px; background: var(--c); }
  .slot-body { display: grid; gap: 14px; padding: 4px 14px 16px; }
  .slot-body .btn.danger { justify-self: start; }
  .sticky { position: sticky; top: 84px; display: grid; gap: 12px; max-height: calc(100vh - 104px); }
  .pv { list-style: none; margin: 0; padding: 0; overflow-y: auto; display: grid; gap: 2px; max-height: 55vh; }
  .pv li { display: grid; grid-template-columns: 70px 1fr auto; gap: 8px; padding: 7px 4px; border-bottom: 1px solid var(--line); font-size: 13px; align-items: baseline; }
  .pv li.filler { opacity: 0.5; }
  .tm { font-variant-numeric: tabular-nums; color: var(--text-2); font-weight: 650; }
  .pt { min-width: 0; }
  .psub { display: block; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .pt .badge { margin-left: 4px; }
  .save { display: flex; gap: 8px; align-items: center; padding-top: 8px; border-top: 1px solid var(--line); }
  .spacer { flex: 1; }
  @media (max-width: 980px) {
    .layout { grid-template-columns: 1fr; }
    .sticky { position: static; max-height: none; }
  }
  @media (max-width: 600px) {
    .ident { flex-direction: column; align-items: stretch; }
    .ident :global(.badge-ch) { align-self: center; }
    .slot-body .two { grid-template-columns: 1fr 1fr; }
    .slot-body .two .field:first-child { grid-column: span 2; }
  }
</style>

<script lang="ts">
  import type { Slot } from '../lib/api';
  import { DAYS } from '../lib/time';

  let { slots, colors, onpick }: { slots: Slot[]; colors: string[]; onpick?: (i: number) => void } = $props();

  // Split each block into per-day segments, carrying overnight blocks into the next day.
  const rows = $derived(
    DAYS.map((_, d) => {
      const segs: { i: number; from: number; to: number }[] = [];
      slots.forEach((s, i) => {
        const overnight = s.end_min <= s.start_min;
        if (s.days & (1 << d)) segs.push({ i, from: s.start_min, to: overnight ? 1440 : s.end_min });
        const prev = (d + 6) % 7;
        if (overnight && s.days & (1 << prev) && s.end_min > 0) segs.push({ i, from: 0, to: s.end_min });
      });
      return segs;
    }),
  );
</script>

<div class="wt">
  <div class="hours"><span></span>{#each [0, 6, 12, 18] as h}<span style:left="{(h / 24) * 100}%">{h === 0 ? '12a' : h === 12 ? '12p' : h > 12 ? `${h - 12}p` : `${h}a`}</span>{/each}</div>
  {#each rows as segs, d}
    <div class="day">
      <span class="lbl">{DAYS[d]}</span>
      <div class="track">
        {#each [6, 12, 18] as h}<i class="grid" style:left="{(h / 24) * 100}%"></i>{/each}
        {#each segs as s}
          <button type="button" class="seg" style:left="{(s.from / 1440) * 100}%" style:width="{((s.to - s.from) / 1440) * 100}%" style:--c={colors[s.i % colors.length]}
            title={slots[s.i].title} onclick={() => onpick?.(s.i)}>
            <span>{slots[s.i].title}</span>
          </button>
        {/each}
      </div>
    </div>
  {/each}
</div>

<style>
  .wt { display: grid; gap: 4px; }
  .hours { position: relative; height: 16px; margin-left: 44px; font-size: 11px; color: var(--text-3); }
  .hours span { position: absolute; }
  .day { display: flex; align-items: center; gap: 8px; }
  .lbl { width: 36px; font-size: 12px; font-weight: 700; color: var(--text-2); }
  .track { position: relative; flex: 1; height: 26px; border-radius: 7px; background: var(--bg-2); border: 1px solid var(--line); overflow: hidden; }
  .grid { position: absolute; top: 0; bottom: 0; width: 1px; background: var(--line); }
  .seg { position: absolute; top: 2px; bottom: 2px; border: 0; border-radius: 5px; padding: 0 6px; background: color-mix(in srgb, var(--c) 70%, #000); color: #fff; font-size: 11px; font-weight: 700; overflow: hidden; text-align: left; }
  .seg span { white-space: nowrap; }
</style>

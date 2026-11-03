<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { api, type Program } from '../lib/api';
  import { go } from '../lib/state.svelte';
  import { playFrom } from '../lib/hls';
  import { range } from '../lib/time';

  // Watch one program from its start, with normal seek/pause controls.
  let { programId }: { programId: number } = $props();

  let video: HTMLVideoElement;
  let program = $state<Program | null>(null);
  let channelId = $state<number | null>(null);
  let error = $state('');
  let kept: [number, number][] = [];
  let base = 0;
  let session = '';
  let destroy = () => {};

  // Same as live TV: jump over skipped intros/credits.
  function skipCuts() {
    if (kept.length < 2 && !(kept.length === 1 && kept[0][0] > 0)) return;
    const media = base + video.currentTime;
    const i = kept.findIndex(([, e]) => media < e - 0.25);
    if (i < 0) return;
    if (media < kept[i][0] - 0.5) video.currentTime = kept[i][0] - base;
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === 'Escape' || e.key === 'Backspace') {
      e.preventDefault();
      backToLive();
    } else if (e.key === ' ') {
      e.preventDefault();
      video.paused ? video.play() : video.pause();
    } else if (e.key === 'ArrowLeft' || e.key === 'ArrowRight') {
      e.preventDefault();
      video.currentTime = Math.max(0, video.currentTime + (e.key === 'ArrowLeft' ? -10 : 30));
    }
  }

  function backToLive() {
    go(channelId !== null ? `watch/${channelId}` : 'guide');
  }

  onMount(async () => {
    try {
      const r = await api.replay(programId);
      program = r.program;
      channelId = r.channel_id;
      session = r.session;
      kept = r.kept;
      const p = await playFrom(video, r.url, r.offset, r.duration);
      base = p.base;
      destroy = p.destroy;
      video.play().catch(() => {
        video.muted = true;
        video.play().catch(() => {});
      });
    } catch (e) {
      error = (e as Error).message;
    }
  });

  onDestroy(() => {
    destroy();
    if (session) api.stopSession(session);
  });
</script>

<svelte:window onkeydown={onKey} />

<div class="replay">
  <!-- svelte-ignore a11y_media_has_caption -->
  <video bind:this={video} controls playsinline ontimeupdate={skipCuts}></video>
  <div class="top">
    <button class="btn" onclick={backToLive}>● Back to live</button>
    {#if program}
      <div class="t">
        <span class="badge">FROM THE START</span>
        <b>{program.title}</b>
        {#if program.subtitle}<span class="muted">{program.subtitle}</span>{/if}
        <span class="dim small">aired {range(program.start, program.end)}</span>
      </div>
    {/if}
  </div>
  {#if error}
    <div class="err"><p>{error}</p><button class="btn" onclick={() => go('guide')}>Back to the guide</button></div>
  {/if}
</div>

<style>
  .replay { position: fixed; inset: 0; background: #000; }
  video { position: absolute; inset: 0; width: 100%; height: 100%; object-fit: contain; }
  .top { position: absolute; top: 0; left: 0; right: 0; display: flex; align-items: center; gap: 16px; padding: 14px 18px; background: linear-gradient(180deg, rgb(0 0 0 / 0.75), transparent); }
  .t { display: flex; align-items: baseline; gap: 10px; flex-wrap: wrap; min-width: 0; }
  .t b { font-size: 18px; }
  .err { position: absolute; inset: 0; display: grid; place-content: center; justify-items: center; gap: 14px; text-align: center; }
</style>

<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { api } from '../lib/api';
  import { playFrom } from '../lib/hls';
  import { mmss } from '../lib/time';
  import Modal from './Modal.svelte';

  // Plays an episode around a marker so you can check it or set exact times.
  let { itemId, title, at, onmark, onclose }: {
    itemId: number; title: string; at: number;
    onmark: (which: 'start' | 'end', ms: number) => void; onclose: () => void;
  } = $props();

  let video: HTMLVideoElement;
  let base = 0;
  let pos = $state(0);
  let error = $state('');
  let session = '';
  let destroy = () => {};

  async function open(fromMs: number) {
    destroy();
    if (session) api.stopSession(session);
    try {
      const p = await api.playItem(itemId, fromMs);
      session = p.session;
      const r = await playFrom(video, p.url, p.offset, p.duration);
      base = r.base;
      destroy = r.destroy;
      video.play().catch(() => {});
    } catch (e) {
      error = (e as Error).message;
    }
  }

  function nudge(sec: number) {
    video.currentTime = Math.max(0, video.currentTime + sec);
  }

  const now = () => Math.round((base + video.currentTime) * 1000);

  onMount(() => open(Math.max(0, at - 5000)));
  onDestroy(() => {
    destroy();
    if (session) api.stopSession(session);
  });
</script>

<Modal title="Preview — {title}" {onclose} wide>
  <!-- svelte-ignore a11y_media_has_caption -->
  <video bind:this={video} controls playsinline ontimeupdate={() => (pos = base + video.currentTime)}></video>
  {#if error}<p class="err small">{error}</p>{/if}
  <div class="row wrap ctl">
    <span class="time">{mmss(pos * 1000)}</span>
    <button class="btn small" onclick={() => nudge(-5)}>−5s</button>
    <button class="btn small" onclick={() => nudge(-1)}>−1s</button>
    <button class="btn small" onclick={() => nudge(1)}>+1s</button>
    <button class="btn small" onclick={() => nudge(5)}>+5s</button>
    <div class="spacer"></div>
    <button class="btn small primary" onclick={() => onmark('start', now())}>Mark start here</button>
    <button class="btn small primary" onclick={() => onmark('end', now())}>Mark end here</button>
  </div>
  <p class="tiny dim">Play to where the intro (or credits) begins and press <b>Mark start</b>, then where it ends and press <b>Mark end</b>.</p>
</Modal>

<style>
  video { width: 100%; aspect-ratio: 16/9; background: #000; border-radius: 10px; }
  .ctl { margin: 10px 0 6px; gap: 6px; }
  .time { font-family: var(--display); font-size: 20px; min-width: 80px; color: var(--accent); }
  .spacer { flex: 1; }
  .err { color: var(--err); }
</style>

<script lang="ts">
  import { onMount } from 'svelte';

  // Old-school analog static, shown briefly while changing channels.
  let canvas: HTMLCanvasElement;

  onMount(() => {
    const ctx = canvas.getContext('2d');
    if (!ctx) return;
    const w = (canvas.width = 320);
    const h = (canvas.height = 180);
    const img = ctx.createImageData(w, h);
    let raf = 0;
    const draw = () => {
      const d = img.data;
      for (let i = 0; i < d.length; i += 4) {
        const v = (Math.random() * 255) | 0;
        d[i] = d[i + 1] = d[i + 2] = v;
        d[i + 3] = 255;
      }
      ctx.putImageData(img, 0, 0);
      raf = requestAnimationFrame(draw);
    };
    draw();
    return () => cancelAnimationFrame(raf);
  });
</script>

<canvas bind:this={canvas} aria-hidden="true"></canvas>

<style>
  canvas { position: absolute; inset: 0; width: 100%; height: 100%; image-rendering: pixelated; opacity: 0.85; filter: contrast(1.2); }
</style>

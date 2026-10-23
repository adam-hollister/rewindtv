<script lang="ts">
  import type { Snippet } from 'svelte';
  let { title, onclose, children, footer, wide = false }: { title: string; onclose: () => void; children: Snippet; footer?: Snippet; wide?: boolean } = $props();
</script>

<svelte:window onkeydown={(e) => e.key === 'Escape' && onclose()} />

<div class="back" role="presentation" onclick={onclose}>
  <div class="modal card" class:wide role="dialog" aria-modal="true" aria-label={title} tabindex="-1" onclick={(e) => e.stopPropagation()} onkeydown={() => {}}>
    <header><h2>{title}</h2><button class="btn ghost icon" onclick={onclose} aria-label="Close">✕</button></header>
    <div class="body">{@render children()}</div>
    {#if footer}<footer>{@render footer()}</footer>{/if}
  </div>
</div>

<style>
  .back { position: fixed; inset: 0; z-index: 60; background: rgb(0 0 0 / 0.6); backdrop-filter: blur(4px); display: grid; place-items: center; padding: 16px; animation: f 0.2s; }
  @keyframes f { from { opacity: 0; } }
  .modal { width: min(560px, 100%); max-height: calc(100vh - 32px); display: flex; flex-direction: column; box-shadow: var(--shadow); animation: up 0.22s ease; }
  .modal.wide { width: min(880px, 100%); }
  @keyframes up { from { transform: translateY(14px); } }
  header { display: flex; align-items: center; justify-content: space-between; padding: 18px 20px 12px; }
  .body { padding: 4px 20px 20px; overflow-y: auto; }
  footer { display: flex; gap: 10px; justify-content: flex-end; padding: 14px 20px; border-top: 1px solid var(--line); }
</style>

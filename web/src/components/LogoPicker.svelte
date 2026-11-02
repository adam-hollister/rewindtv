<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import { api, type ArtChoice } from '../lib/api';
  import { toast } from '../lib/state.svelte';
  import ChannelBadge from './ChannelBadge.svelte';
  import Modal from './Modal.svelte';

  let { channelId, name, number, color, logo, onchange, onclose }: {
    channelId: number; name: string; number: number; color: string; logo: string | null;
    onchange: (logo: string | null) => void; onclose: () => void;
  } = $props();

  // Starts from the channel's logo, then follows changes made here.
  let current = $state(untrack(() => logo));
  let choices = $state<ArtChoice[] | null>(null);
  let busy = $state(false);
  let fileInput: HTMLInputElement;

  onMount(async () => {
    choices = await api.artChoices(channelId).catch(() => []);
  });

  async function apply(p: Promise<{ logo: string | null }>, ok: string) {
    busy = true;
    try {
      current = (await p).logo;
      onchange(current);
      toast(ok);
    } catch (e) {
      toast((e as Error).message, 'err');
    } finally {
      busy = false;
    }
  }

  function upload(e: Event) {
    const f = (e.currentTarget as HTMLInputElement).files?.[0];
    if (!f) return;
    if (f.size > 3 * 1024 * 1024) return toast('Logos can be up to 3 MB', 'err');
    apply(api.uploadLogo(channelId, f), 'Logo uploaded');
    fileInput.value = '';
  }
</script>

<Modal title="Logo for {name}" {onclose} wide>
  <div class="top">
    <div class="preview">
      <ChannelBadge {number} {color} id={channelId} logo={current} size="xl" />
      <div class="row-mock">
        <ChannelBadge {number} {color} id={channelId} logo={current} />
        <span>{name}</span>
      </div>
      <span class="tiny dim">How it looks in the guide — and in Plex's Live TV guide.</span>
    </div>
    <div class="actions">
      <input bind:this={fileInput} type="file" accept="image/png,image/jpeg,image/webp" hidden onchange={upload} />
      <button class="btn primary" onclick={() => fileInput.click()} disabled={busy}>Upload image…</button>
      <span class="tiny dim">PNG, JPEG or WebP, up to 3 MB. A transparent PNG looks best.</span>
      {#if current}
        <button class="btn ghost" onclick={() => apply(api.clearLogo(channelId), 'Back to the number badge')} disabled={busy}>Use number badge</button>
      {/if}
    </div>
  </div>

  <h3>Or pick artwork from this channel's shows</h3>
  {#if choices === null}
    <p class="muted small">Loading artwork…</p>
  {:else if choices.length === 0}
    <p class="muted small">No artwork found — check the channel's rules match something.</p>
  {:else}
    <div class="grid">
      {#each choices as c (c.source_id + (c.poster ?? c.backdrop ?? c.title))}
        {#if c.backdrop_url}
          <button class="art wide" title="{c.title} (backdrop)" disabled={busy} onclick={() => apply(api.logoFromArt(channelId, c.source_id, c.backdrop!), `Using ${c.title}`)}>
            <img src={c.backdrop_url} alt="" loading="lazy" /><span>{c.title}</span>
          </button>
        {/if}
        {#if c.poster_url}
          <button class="art" title="{c.title} (poster)" disabled={busy} onclick={() => apply(api.logoFromArt(channelId, c.source_id, c.poster!), `Using ${c.title}`)}>
            <img src={c.poster_url} alt="" loading="lazy" />
          </button>
        {/if}
      {/each}
    </div>
  {/if}
</Modal>

<style>
  .top { display: flex; gap: 28px; align-items: center; flex-wrap: wrap; margin-bottom: 18px; }
  .preview { display: grid; gap: 14px; justify-items: start; }
  .row-mock { display: flex; align-items: center; gap: 12px; padding: 10px 14px; border-radius: 10px; background: var(--bg-2); border: 1px solid var(--line); font-weight: 700; min-width: 220px; }
  .actions { display: grid; gap: 8px; justify-items: start; max-width: 300px; }
  h3 { margin: 6px 0 10px; }
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(92px, 1fr)); gap: 8px; max-height: 340px; overflow-y: auto; }
  .art { position: relative; padding: 0; border: 1px solid var(--line); border-radius: 10px; overflow: hidden; background: var(--bg-2); aspect-ratio: 2/3; }
  .art.wide { grid-column: span 2; aspect-ratio: auto; }
  .art img { width: 100%; height: 100%; object-fit: cover; display: block; }
  .art span { position: absolute; left: 0; right: 0; bottom: 0; padding: 4px 6px; font-size: 11px; font-weight: 700; background: linear-gradient(0deg, rgb(0 0 0 / 0.8), transparent); text-align: left; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .art:hover, .art:focus-visible { border-color: var(--accent); }
</style>

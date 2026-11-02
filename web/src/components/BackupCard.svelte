<script lang="ts">
  import { onMount } from 'svelte';
  import { api, type BackupFile } from '../lib/api';
  import { toast } from '../lib/state.svelte';
  import Modal from './Modal.svelte';

  let dir = $state('');
  let files = $state<BackupFile[]>([]);
  let confirm = $state<{ label: string; run: () => Promise<unknown> } | null>(null);
  let busy = $state(false);
  let fileInput: HTMLInputElement;

  async function load() {
    const r = await api.backups();
    dir = r.dir;
    files = r.files;
  }
  onMount(load);

  function pickFile(e: Event) {
    const f = (e.currentTarget as HTMLInputElement).files?.[0];
    fileInput.value = '';
    if (!f) return;
    confirm = { label: `the file “${f.name}”`, run: () => api.restoreBackupFile(f) };
  }

  async function doRestore() {
    if (!confirm) return;
    busy = true;
    try {
      await confirm.run();
      confirm = null;
      toast('Restored — reconnecting to your servers and rebuilding guides…');
      // Profiles and channels changed under the page; start fresh.
      setTimeout(() => location.reload(), 2500);
    } catch (e) {
      toast((e as Error).message, 'err');
    } finally {
      busy = false;
    }
  }

  const when = (ms: number) => new Date(ms).toLocaleString(undefined, { weekday: 'short', month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit' });
  const size = (b: number) => (b > 1048576 ? `${(b / 1048576).toFixed(1)} MB` : `${Math.max(1, Math.round(b / 1024))} KB`);
</script>

<section class="card pad">
  <h2>Backup &amp; restore</h2>
  <p class="muted small sub">One file with your profiles, servers, channels, logos, intro/credits markers and settings. RewindTV also saves one automatically every night and keeps two weeks of them. The guide rebuilds itself after a restore.</p>
  <div class="row wrap">
    <a class="btn primary" href="/api/backup" download>Download backup</a>
    <input bind:this={fileInput} type="file" accept="application/json,.json" hidden onchange={pickFile} />
    <button class="btn" onclick={() => fileInput.click()}>Restore from file…</button>
  </div>
  <p class="tiny dim warn">Backups include your Plex/Jellyfin sign-in so a restore reconnects by itself — keep them somewhere private.</p>

  <h3>Automatic backups</h3>
  {#if files.length === 0}
    <p class="small muted">None yet — the first one is made shortly after RewindTV starts.</p>
  {:else}
    <div class="list">
      {#each files as f (f.name)}
        <div class="item">
          <span class="when"><b>{when(f.modified)}</b><span class="tiny dim">{f.automatic ? 'Nightly' : 'Saved before a restore'} · {size(f.bytes)}</span></span>
          <a class="btn small ghost" href="/api/backups/{encodeURIComponent(f.name)}" download>Download</a>
          <button class="btn small" onclick={() => (confirm = { label: `the backup from ${when(f.modified)}`, run: () => api.restoreBackup(f.name) })}>Restore</button>
        </div>
      {/each}
    </div>
  {/if}
  {#if dir}<p class="tiny dim">Saved in <code>{dir}</code></p>{/if}
</section>

{#if confirm}
  <Modal title="Restore?" onclose={() => (confirm = null)}>
    <p>Restore from {confirm.label}?</p>
    <p class="muted small">This replaces your profiles, servers, channels, logos and markers with the ones in the backup. Your current setup is saved as a backup first, so you can go back.</p>
    {#snippet footer()}
      <button class="btn ghost" onclick={() => (confirm = null)}>Cancel</button>
      <button class="btn primary" onclick={doRestore} disabled={busy}>{busy ? 'Restoring…' : 'Restore'}</button>
    {/snippet}
  </Modal>
{/if}

<style>
  .pad { padding: 22px; }
  .sub { margin: 6px 0 14px; }
  .warn { margin-top: 10px; }
  h3 { margin: 18px 0 8px; }
  .list { display: grid; gap: 4px; margin-bottom: 8px; }
  .item { display: flex; align-items: center; gap: 8px; padding: 8px 10px; border-radius: 10px; background: var(--bg-2); border: 1px solid var(--line); }
  .when { flex: 1; display: grid; }
  code { font-size: 12px; color: var(--text-2); overflow-wrap: anywhere; }
</style>

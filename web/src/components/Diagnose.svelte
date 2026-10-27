<script lang="ts">
  import { onMount } from 'svelte';
  import { api, type DiagnoseReport } from '../lib/api';
  import { toast } from '../lib/state.svelte';
  import Modal from './Modal.svelte';

  let { id, name, onclose }: { id: number; name: string; onclose: () => void } = $props();
  let report = $state<DiagnoseReport | null>(null);
  let error = $state('');

  const STEPS = ['Connect to server', 'Library sync', 'Artwork', 'Browser playback', 'Live TV transcode', 'Direct file access', 'Intro & credits markers', 'Audio fingerprinting', 'ffmpeg'];

  async function run() {
    report = null;
    error = '';
    try {
      report = await api.diagnose(id);
    } catch (e) {
      error = (e as Error).message;
    }
  }
  onMount(run);

  function copy() {
    if (!report) return;
    const text = [`RewindTV check — ${report.source} (${report.kind})`, report.item ? `Test item: ${report.item}` : '',
      ...report.checks.map((c) => `${c.ok ? 'PASS' : 'FAIL'}  ${c.name} (${c.ms} ms): ${c.detail}`)].filter(Boolean).join('\n');
    navigator.clipboard?.writeText(text).then(() => toast('Report copied'), () => prompt('Copy this report', text));
  }
</script>

<Modal title="Check {name}" {onclose}>
  {#if error}
    <p class="err">{error}</p>
  {:else if !report}
    <p class="muted small">Testing with a real item from your library. Transcoding checks can take up to a minute…</p>
    <ul class="checks">
      {#each STEPS as s}<li><span class="ic run"></span><span>{s}</span></li>{/each}
    </ul>
  {:else}
    <div class="summary" class:ok={report.ok}>
      {report.ok ? '✓ Everything works' : '✕ Something needs attention'}
      {#if report.item}<span class="dim small"> · tested with “{report.item}”</span>{/if}
    </div>
    <ul class="checks">
      {#each report.checks as c}
        <li class:fail={!c.ok}>
          <span class="ic">{c.ok ? '✓' : '✕'}</span>
          <span class="nm">{c.name}</span>
          <span class="dt small">{c.detail}</span>
          <span class="ms tiny dim">{c.ms < 1000 ? `${c.ms} ms` : `${(c.ms / 1000).toFixed(1)} s`}</span>
        </li>
      {/each}
    </ul>
  {/if}
  {#snippet footer()}
    <button class="btn ghost" onclick={copy} disabled={!report}>Copy report</button>
    <button class="btn" onclick={run} disabled={!report && !error}>Run again</button>
    <button class="btn primary" onclick={onclose}>Done</button>
  {/snippet}
</Modal>

<style>
  .checks { list-style: none; margin: 12px 0 0; padding: 0; display: grid; gap: 6px; }
  li { display: grid; grid-template-columns: 24px 150px 1fr auto; gap: 10px; align-items: baseline; padding: 10px 12px; border-radius: 10px; background: var(--bg-2); border: 1px solid var(--line); }
  li.fail { border-color: rgb(248 113 113 / 0.45); background: rgb(248 113 113 / 0.06); }
  .ic { font-weight: 900; color: var(--ok); }
  li.fail .ic { color: var(--err); }
  .ic.run { width: 14px; height: 14px; border-radius: 50%; border: 2px solid var(--line-2); border-top-color: var(--accent); animation: spin 0.9s linear infinite; align-self: center; }
  @keyframes spin { to { transform: rotate(360deg); } }
  .nm { font-weight: 700; font-size: 14px; }
  .dt { color: var(--text-2); overflow-wrap: anywhere; }
  .summary { font-weight: 800; color: #fca5a5; }
  .summary.ok { color: var(--ok); }
  .err { color: var(--err); }
  @media (max-width: 560px) {
    li { grid-template-columns: 24px 1fr auto; }
    .dt { grid-column: 2 / 4; }
  }
</style>

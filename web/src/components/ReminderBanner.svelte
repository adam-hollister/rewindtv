<script lang="ts">
  import { onMount } from 'svelte';
  import { reminders, refreshReminders, route, go, session } from '../lib/state.svelte';

  // Shows "starting soon" for reminders anywhere in the app. In TV mode it
  // switches channel after a short countdown unless dismissed.
  let now = $state(Date.now());
  let dismissed = $state(new Set<number>());
  let countdown = $state(10);

  onMount(() => {
    refreshReminders();
    const tick = setInterval(() => (now = Date.now()), 1000);
    const poll = setInterval(refreshReminders, 30_000);
    return () => {
      clearInterval(tick);
      clearInterval(poll);
    };
  });

  $effect(() => {
    session.userId;
    refreshReminders();
  });

  const due = $derived(
    reminders.list.find((r) => !dismissed.has(r.program_id) && r.start_ms - now <= 60_000 && now - r.start_ms < 5 * 60_000),
  );
  const watching = $derived(route.parts[0] === 'watch');
  const already = $derived(due && watching && Number(route.parts[1]) === due.channel_id);

  // Countdown to switching, only in TV mode.
  $effect(() => {
    if (!due || !watching || already) return;
    countdown = 10;
    const t = setInterval(() => {
      countdown -= 1;
      if (countdown <= 0) {
        clearInterval(t);
        watch();
      }
    }, 1000);
    return () => clearInterval(t);
  });

  function dismiss() {
    if (due) dismissed = new Set([...dismissed, due.program_id]);
  }

  function watch() {
    if (!due) return;
    const ch = due.channel_id;
    dismiss();
    go(`watch/${ch}`);
  }

  const when = (r: { start_ms: number }) => {
    const s = Math.round((r.start_ms - now) / 1000);
    return s > 5 ? `starts in ${s >= 60 ? '1 min' : `${s}s`}` : 'is starting now';
  };
</script>

{#if due && !already}
  <div class="banner" role="alert">
    <span class="bell">🔔</span>
    <span class="txt"><b>{due.title}</b> {when(due)}</span>
    <button class="btn small primary" onclick={watch}>Watch{watching ? ` (${countdown})` : ''}</button>
    <button class="btn small ghost" onclick={dismiss}>Dismiss</button>
  </div>
{/if}

<style>
  .banner {
    position: fixed; top: 16px; left: 50%; transform: translateX(-50%); z-index: 90;
    display: flex; align-items: center; gap: 12px; padding: 12px 16px; max-width: calc(100vw - 32px);
    background: rgb(21 25 34 / 0.95); backdrop-filter: blur(12px); border: 1px solid rgb(245 165 36 / 0.6);
    border-radius: 14px; box-shadow: var(--shadow); animation: drop 0.3s ease;
  }
  @keyframes drop { from { transform: translate(-50%, -20px); opacity: 0; } }
  .bell { font-size: 20px; }
  .txt { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
</style>

<script lang="ts">
  import { api } from '../lib/api';
  import { session, selectUser, loadUsers, go, toast } from '../lib/state.svelte';

  const COLORS = ['#f5a524', '#ef4444', '#10b981', '#3b82f6', '#8b5cf6', '#ec4899', '#14b8a6', '#f97316'];
  let adding = $state(false);
  let name = $state('');
  let color = $state(COLORS[0]);

  $effect(() => {
    if (session.loaded && session.users.length === 0) adding = true;
  });

  function pick(id: number) {
    selectUser(id);
    go('guide');
  }

  async function add(e: Event) {
    e.preventDefault();
    if (!name.trim()) return;
    try {
      const u = await api.createUser({ name: name.trim(), color });
      await loadUsers();
      name = '';
      adding = false;
      pick(u.id);
    } catch (err) {
      toast((err as Error).message, 'err');
    }
  }
</script>

<main>
  <div class="logo">REWIND<b>TV</b></div>
  {#if adding}
    <form class="card add" onsubmit={add}>
      <h2>{session.users.length ? 'Add a profile' : 'Welcome! Create your profile'}</h2>
      <p class="muted small">Profiles keep their own channel lineup and favorites. No passwords — this is your living room.</p>
      <!-- svelte-ignore a11y_autofocus -->
      <label class="field">Name<input type="text" bind:value={name} placeholder="e.g. Mom, Kids, Adam" autofocus /></label>
      <div class="colors">
        {#each COLORS as c}
          <button type="button" class="sw" class:on={c === color} style:background={c} onclick={() => (color = c)} aria-label="Color {c}"></button>
        {/each}
      </div>
      <div class="row">
        {#if session.users.length}<button type="button" class="btn ghost" onclick={() => (adding = false)}>Cancel</button>{/if}
        <div class="spacer"></div>
        <button class="btn primary" disabled={!name.trim()}>Continue</button>
      </div>
    </form>
  {:else}
    <h1>Who's watching?</h1>
    <div class="grid">
      {#each session.users as u (u.id)}
        <button class="profile" onclick={() => pick(u.id)}>
          <span class="av" style:--c={u.color}>{u.name[0]?.toUpperCase()}</span>
          <span class="nm">{u.name}</span>
        </button>
      {/each}
      <button class="profile" onclick={() => (adding = true)}>
        <span class="av plus">+</span>
        <span class="nm muted">Add profile</span>
      </button>
    </div>
  {/if}
</main>

<style>
  main { min-height: 100vh; display: grid; place-content: center; justify-items: center; gap: 36px; padding: 24px; text-align: center; }
  .logo { font-family: var(--display); font-size: clamp(36px, 7vw, 64px); letter-spacing: 0.04em; text-shadow: 0 6px 40px rgb(245 165 36 / 0.25); }
  .logo b { color: var(--accent); font-weight: inherit; }
  h1 { font-size: clamp(24px, 4vw, 38px); }
  .grid { display: flex; flex-wrap: wrap; justify-content: center; gap: 28px; }
  .profile { display: grid; justify-items: center; gap: 12px; border: 0; background: none; padding: 8px; border-radius: 20px; }
  .av {
    width: 132px; height: 132px; border-radius: 28px; display: grid; place-items: center;
    font-size: 56px; font-weight: 850; color: rgb(0 0 0 / 0.75);
    background: linear-gradient(145deg, var(--c), color-mix(in srgb, var(--c) 55%, #000));
    box-shadow: 0 20px 40px -18px var(--c);
    transition: transform 0.18s, box-shadow 0.18s;
  }
  .av.plus { background: var(--panel); border: 2px dashed var(--line-2); color: var(--text-3); box-shadow: none; }
  .profile:hover .av, .profile:focus-visible .av { transform: translateY(-4px) scale(1.04); box-shadow: 0 0 0 4px var(--text), 0 24px 50px -18px var(--c); }
  .profile:focus-visible { outline: none; }
  .nm { font-weight: 700; font-size: 17px; }
  .add { width: min(440px, 100%); padding: 28px; display: grid; gap: 18px; text-align: left; }
  .colors { display: flex; gap: 10px; flex-wrap: wrap; }
  .sw { width: 34px; height: 34px; border-radius: 50%; border: 0; }
  .sw.on { box-shadow: 0 0 0 3px var(--bg), 0 0 0 5px var(--text); }
  @media (max-width: 640px) { .av { width: 96px; height: 96px; font-size: 40px; border-radius: 22px; } }
</style>

<script lang="ts">
  let {
    number,
    color,
    size = 'md',
    id = null,
    logo = null,
  }: { number: number; color: string; size?: 'md' | 'lg' | 'xl'; id?: number | null; logo?: string | null } = $props();
</script>

{#if logo && id !== null}
  <span class="badge-ch logo {size}" style:--c={color}>
    <img src="/api/channels/{id}/logo?v={encodeURIComponent(logo)}" alt="" loading="lazy" />
    <i>{number}</i>
  </span>
{:else}
  <span class="badge-ch {size}" style:--c={color}>{number}</span>
{/if}

<style>
  .badge-ch {
    flex: none; display: grid; place-items: center;
    width: 46px; height: 34px; border-radius: 9px;
    font-family: var(--display); font-size: 16px; color: rgb(0 0 0 / 0.82);
    background: linear-gradient(150deg, color-mix(in srgb, var(--c) 100%, #fff 10%), color-mix(in srgb, var(--c) 70%, #000));
    box-shadow: inset 0 1px 0 rgb(255 255 255 / 0.3), 0 6px 16px -8px var(--c);
  }
  .lg { width: 72px; height: 54px; font-size: 26px; border-radius: 13px; }
  .xl { width: 104px; height: 76px; font-size: 40px; border-radius: 18px; }

  .logo { position: relative; background: #0d1017; box-shadow: inset 0 0 0 1px rgb(255 255 255 / 0.08), 0 6px 16px -8px var(--c); overflow: visible; }
  .logo img { width: 100%; height: 100%; object-fit: contain; border-radius: inherit; padding: 3px; }
  .logo i {
    position: absolute; right: -6px; bottom: -6px; min-width: 20px; height: 18px; padding: 0 5px;
    display: grid; place-items: center; border-radius: 6px; font-style: normal;
    font-family: var(--display); font-size: 11px; color: rgb(0 0 0 / 0.85); background: var(--c);
    box-shadow: 0 0 0 2px var(--bg);
  }
  .lg.logo i { font-size: 13px; height: 22px; min-width: 24px; }
  .xl.logo i { font-size: 16px; height: 26px; min-width: 30px; right: -8px; bottom: -8px; }
</style>

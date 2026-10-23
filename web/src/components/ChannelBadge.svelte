<script lang="ts">
  let { number, color, size = 'md' }: { number: number; color: string; size?: 'md' | 'lg' | 'xl' } = $props();
</script>

<span class="badge-ch {size}" style:--c={color}>{number}</span>

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
</style>

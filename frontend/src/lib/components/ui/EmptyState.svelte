<script lang="ts">
  import type { ComponentType } from 'svelte';
  export let icon: ComponentType | null = null;
  export let title: string;
  export let description = '';
</script>

<div class="flex flex-col items-center justify-center text-center px-6 py-14">
  {#if icon}
    <div class="w-12 h-12 rounded-2xl bg-muted text-muted-foreground flex items-center justify-center mb-3">
      <svelte:component this={icon} size={22} />
    </div>
  {/if}
  <p class="text-sm font-semibold">{title}</p>
  {#if description}<p class="text-sm text-muted-foreground mt-1 max-w-sm">{description}</p>{/if}
  <slot />
</div>

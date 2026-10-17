<script lang="ts">
  import { createEventDispatcher } from 'svelte';
  import { fade, fly } from 'svelte/transition';
  import { X } from 'lucide-svelte';

  export let open = false;
  export let title = '';
  export let size: 'md' | 'lg' = 'md';

  const dispatch = createEventDispatcher<{ close: void }>();
  const close = () => dispatch('close');
</script>

<svelte:window on:keydown={(e) => open && e.key === 'Escape' && close()} />

{#if open}
  <div class="fixed inset-0 z-[60] flex items-end sm:items-center justify-center sm:p-4">
    <div class="absolute inset-0 bg-black/50 backdrop-blur-sm" transition:fade={{ duration: 150 }}
         on:click={close} role="presentation"></div>
    <div
      class="relative w-full {size === 'lg' ? 'sm:max-w-2xl' : 'sm:max-w-md'} max-h-[92dvh] flex flex-col
             bg-card border rounded-t-2xl sm:rounded-2xl shadow-2xl"
      role="dialog" aria-modal="true" aria-label={title}
      transition:fly={{ y: 24, duration: 180 }}
    >
      <div class="flex items-center justify-between gap-3 px-5 pt-5 pb-3">
        <h2 class="text-base font-bold">{title}</h2>
        <button type="button" class="icon-btn -mr-2" on:click={close} aria-label="Close"><X size={18} /></button>
      </div>
      <div class="px-5 pb-5 overflow-y-auto">
        <slot />
      </div>
      {#if $$slots.footer}
        <div class="flex flex-col-reverse sm:flex-row sm:justify-end gap-2 px-5 py-4 border-t bg-muted/30 rounded-b-2xl">
          <slot name="footer" />
        </div>
      {/if}
    </div>
  </div>
{/if}

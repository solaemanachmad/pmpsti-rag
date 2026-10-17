<script lang="ts">
  export let title: string;
  export let description = '';
</script>

<div class="flex flex-col gap-3 sm:flex-row sm:items-end sm:justify-between mb-6">
  <div class="min-w-0">
    <h1 class="text-2xl font-extrabold tracking-tight">{title}</h1>
    {#if description}
      <p class="text-sm text-muted-foreground mt-1">{description}</p>
    {/if}
  </div>
  {#if $$slots.default}
    <div class="flex items-center gap-2 flex-wrap shrink-0">
      <slot />
    </div>
  {/if}
</div>

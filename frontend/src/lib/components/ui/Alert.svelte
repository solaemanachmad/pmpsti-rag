<script lang="ts">
  import { AlertCircle, AlertTriangle, CheckCircle2, Info, X } from 'lucide-svelte';
  import { createEventDispatcher } from 'svelte';

  export let variant: 'error' | 'warning' | 'success' | 'info' = 'error';
  export let dismissible = false;

  const dispatch = createEventDispatcher<{ dismiss: void }>();

  const styles = {
    error:   'bg-destructive/10 border-destructive/25 text-destructive',
    warning: 'bg-warning/10 border-warning/30 text-warning',
    success: 'bg-success/10 border-success/25 text-success',
    info:    'bg-primary/10 border-primary/25 text-primary'
  };
  const icons = { error: AlertCircle, warning: AlertTriangle, success: CheckCircle2, info: Info };
</script>

<div class="flex items-start gap-2.5 border rounded-lg px-3.5 py-2.5 text-sm {styles[variant]}" role={variant === 'error' ? 'alert' : 'status'}>
  <svelte:component this={icons[variant]} size={16} class="shrink-0 mt-0.5" />
  <div class="flex-1 min-w-0 leading-relaxed break-words"><slot /></div>
  {#if dismissible}
    <button type="button" on:click={() => dispatch('dismiss')} class="shrink-0 opacity-70 hover:opacity-100" aria-label="Dismiss">
      <X size={15} />
    </button>
  {/if}
</div>

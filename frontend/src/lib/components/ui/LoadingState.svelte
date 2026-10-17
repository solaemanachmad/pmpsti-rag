<script lang="ts">
  import { Loader2 } from 'lucide-svelte';
  export let label = 'Loading…';
</script>

<div class="flex items-center justify-center gap-2 py-16 text-sm text-muted-foreground">
  <Loader2 size={18} class="animate-spin" /> {label}
</div>

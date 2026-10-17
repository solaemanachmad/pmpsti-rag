<script lang="ts">
  import { onMount } from 'svelte';
  import { Moon, Sun } from 'lucide-svelte';
  import { isDark } from '$lib/theme';

  let className = 'icon-btn';
  export { className as class };

  onMount(isDark.sync);
</script>

<button
  type="button"
  on:click={isDark.toggle}
  class={className}
  title={$isDark ? 'Switch to light mode' : 'Switch to dark mode'}
  aria-label={$isDark ? 'Switch to light mode' : 'Switch to dark mode'}
>
  {#if $isDark}<Sun size={17} />{:else}<Moon size={17} />{/if}
</button>

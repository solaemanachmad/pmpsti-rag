<script lang="ts">
  import { FlaskConical } from 'lucide-svelte';

  /** `onDark` switches the wordmark colour for navy surfaces such as the sidebar. */
  export let onDark = false;
  export let compact = false;
</script>

<span class="flex items-center gap-1 min-w-0">
  <img
    src="/dteti-logo-transparent.png"
    alt="DTETI — Department of Electrical Engineering and Information Technology, UGM"
    class="shrink-0 w-auto object-contain {compact ? 'h-8' : 'h-9'}"
  />
  <!-- The "Beta" badge sits on top of the "Answers" wordmark (absolutely positioned, so headers
       keep their height). -->
  <span class="relative block font-extrabold tracking-tight text-[15px] leading-tight whitespace-nowrap">
    <span
      class="absolute bottom-full left-0 mb-0.5 inline-flex items-center gap-0.5 rounded-full
             px-1.5 py-px text-[9px] font-bold uppercase leading-none tracking-wider shadow-sm
             bg-warning text-white"
      title="Versi beta — aplikasi masih dalam pengembangan"
    >
      <FlaskConical size={9} strokeWidth={3} aria-hidden="true" />
      Beta
    </span>
    <span class={onDark ? 'text-accent' : 'text-warning dark:text-accent'}>Answers</span>
  </span>
</span>

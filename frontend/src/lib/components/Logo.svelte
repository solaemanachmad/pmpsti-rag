<script lang="ts">
  import { FlaskConical } from 'lucide-svelte';

  /** `onDark` switches the wordmark colour for navy surfaces such as the sidebar. */
  export let onDark = false;
  export let compact = false;
</script>

<span class="flex items-center gap-1 min-w-0">
  <!-- The "Beta" badge sits on top of the logo image (absolutely positioned, so headers keep
       their height). The ring matches the surface behind it to cut it out from the logo. -->
  <span class="relative shrink-0">
    <img
      src="/dteti-logo-transparent.png"
      alt="DTETI — Department of Electrical Engineering and Information Technology, UGM"
      class="w-auto object-contain {compact ? 'h-8' : 'h-9'}"
    />
    <span
      class="absolute -top-2 left-1/2 -translate-x-1/2 inline-flex items-center gap-0.5 rounded-full
             px-1.5 py-px text-[9px] font-bold uppercase leading-none tracking-wider shadow-sm ring-2
             bg-warning text-white
             {onDark ? 'ring-sidebar' : 'ring-card'}"
      title="Versi beta — aplikasi masih dalam pengembangan"
    >
      <FlaskConical size={9} strokeWidth={3} aria-hidden="true" />
      Beta
    </span>
  </span>
  <span class="block font-extrabold tracking-tight text-[15px] leading-tight whitespace-nowrap">
    <span class={onDark ? 'text-accent' : 'text-warning dark:text-accent'}>Answers</span>
  </span>
</span>

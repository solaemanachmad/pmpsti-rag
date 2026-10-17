<script lang="ts">
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
  <span class="block font-extrabold tracking-tight text-[15px] leading-tight whitespace-nowrap">
    <span class={onDark ? 'text-accent' : 'text-warning dark:text-accent'}>Answers</span>
  </span>
</span>

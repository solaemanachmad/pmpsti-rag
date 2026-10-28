<script lang="ts">
  // Preview of a cited source. Desktop: shows on hover/focus of a [n] mark, a click opens the
  // source in a new tab. Touch screens (no hover): the first tap shows the preview, which has an
  // "Open source" link. One instance per message list, driven by event delegation.
  import { onMount } from 'svelte';
  import { ExternalLink } from 'lucide-svelte';
  import type { SourceRef } from '$lib/api/client';
  import { formatPages, hostname } from '$lib/format';

  /** the element whose [data-cite] marks this popover serves */
  export let root: HTMLElement | null = null;
  /** sources of the message that contains the mark */
  export let sourcesFor: (mark: HTMLElement) => SourceRef[] | undefined;

  let src: SourceRef | null = null;
  let num = 0;
  let x = 0, y = 0, above = false;
  let pop: HTMLDivElement;
  let hideTimer: ReturnType<typeof setTimeout> | undefined;
  let canHover = true;

  $: url = src?.source_url?.startsWith('http') ? src.source_url : '';

  function show(mark: HTMLElement) {
    const n = Number(mark.dataset.cite);
    const s = sourcesFor(mark)?.[n - 1];
    if (!s) return;
    clearTimeout(hideTimer);
    src = s; num = n;
    const r = mark.getBoundingClientRect();
    const width = Math.min(320, window.innerWidth - 32);
    x = Math.max(16, Math.min(r.left + r.width / 2 - width / 2, window.innerWidth - width - 16));
    above = r.bottom + 180 > window.innerHeight && r.top > 200;
    y = above ? r.top - 8 : r.bottom + 8;
  }
  function hideSoon() {
    clearTimeout(hideTimer);
    hideTimer = setTimeout(() => (src = null), 150);
  }
  const markOf = (e: Event) => (e.target as HTMLElement | null)?.closest?.('[data-cite]') as HTMLElement | null;

  onMount(() => {
    canHover = window.matchMedia('(hover: hover)').matches;
    const over = (e: Event) => { const m = markOf(e); if (m && canHover) show(m); };
    const out = (e: Event) => { if (markOf(e) && canHover) hideSoon(); };
    const click = (e: Event) => {
      const m = markOf(e);
      if (!m) return;
      // touch: first tap previews, the link in the preview opens the source
      if (!canHover || m.tagName !== 'A') { e.preventDefault(); show(m); }
    };
    const away = (e: Event) => { if (src && !pop?.contains(e.target as Node) && !markOf(e)) src = null; };
    const key = (e: KeyboardEvent) => { if (e.key === 'Escape') src = null; };
    const scroll = () => { src = null; };
    root?.addEventListener('mouseover', over);
    root?.addEventListener('mouseout', out);
    root?.addEventListener('focusin', over);
    root?.addEventListener('focusout', out);
    root?.addEventListener('click', click);
    document.addEventListener('pointerdown', away);
    document.addEventListener('keydown', key);
    window.addEventListener('scroll', scroll, true);
    return () => {
      root?.removeEventListener('mouseover', over);
      root?.removeEventListener('mouseout', out);
      root?.removeEventListener('focusin', over);
      root?.removeEventListener('focusout', out);
      root?.removeEventListener('click', click);
      document.removeEventListener('pointerdown', away);
      document.removeEventListener('keydown', key);
      window.removeEventListener('scroll', scroll, true);
    };
  });
</script>

{#if src}
  <div bind:this={pop} role="tooltip"
    on:mouseenter={() => clearTimeout(hideTimer)} on:mouseleave={hideSoon}
    class="fixed z-50 w-[min(20rem,calc(100vw-2rem))] rounded-lg border bg-card text-card-foreground shadow-lg p-3 text-xs"
    style="left:{x}px; top:{y}px; {above ? 'transform: translateY(-100%);' : ''}">
    <div class="flex items-start gap-2">
      <span class="shrink-0 w-5 h-5 rounded-md bg-primary/10 text-primary text-[10px] font-bold flex items-center justify-center">{num}</span>
      <div class="min-w-0 flex-1">
        <p class="font-semibold leading-snug line-clamp-2">{src.title || hostname(src.source_url) || 'Document'}</p>
        <p class="mt-0.5 text-[11px] text-muted-foreground truncate">
          {[hostname(src.source_url), src.pages?.length ? `p. ${formatPages(src.pages)}` : ''].filter(Boolean).join(' · ')}
        </p>
      </div>
    </div>
    {#if src.snippet}
      <p class="mt-2 text-muted-foreground leading-relaxed line-clamp-4">{src.snippet}</p>
    {/if}
    {#if url}
      <a href={url} target="_blank" rel="noopener noreferrer"
        class="mt-2 inline-flex items-center gap-1 font-semibold text-primary hover:underline">
        Open source <ExternalLink size={12} />
      </a>
    {/if}
  </div>
{/if}

<script lang="ts">
  import { ChevronDown, ExternalLink, FileText } from 'lucide-svelte';
  import type { SourceRef } from '$lib/api/client';
  import { formatPages, hostname } from '$lib/format';

  export let sources: SourceRef[];
  export let open = false;
</script>

<details class="mt-3 group" bind:open>
  <summary class="inline-flex items-center gap-1.5 text-xs font-semibold text-muted-foreground
                  hover:text-foreground cursor-pointer select-none list-none
                  px-2.5 py-1 rounded-full border bg-card transition-colors">
    <FileText size={12} />
    {sources.length} {sources.length === 1 ? 'source' : 'sources'}
    <ChevronDown size={12} class="transition-transform group-open:rotate-180" />
  </summary>

  <ol class="mt-2 grid gap-2 sm:grid-cols-2">
    {#each sources as src, i}
      {@const hasUrl = src.source_url?.startsWith('http')}
      {@const domain = hostname(src.source_url)}
      <li>
        <svelte:element
          this={hasUrl ? 'a' : 'div'}
          href={hasUrl ? src.source_url : undefined}
          target={hasUrl ? '_blank' : undefined}
          rel={hasUrl ? 'noopener noreferrer' : undefined}
          class="group/src flex items-start gap-2.5 h-full p-3 rounded-lg border bg-card
                 {hasUrl ? 'hover:border-primary/40 hover:bg-primary/5' : ''} transition-colors"
        >
          <span class="shrink-0 w-5 h-5 rounded-md bg-primary/10 text-primary text-[10px] font-bold flex items-center justify-center mt-0.5">
            {i + 1}
          </span>
          <span class="flex-1 min-w-0">
            <span class="flex items-start justify-between gap-2">
              <span class="text-xs font-semibold line-clamp-2 leading-snug">{src.title || domain || 'Document'}</span>
              {#if hasUrl}
                <ExternalLink size={11} class="shrink-0 mt-0.5 text-muted-foreground group-hover/src:text-primary" />
              {/if}
            </span>
            {#if src.snippet}
              <span class="block text-xs text-muted-foreground mt-1 line-clamp-2 leading-relaxed">{src.snippet}</span>
            {/if}
            {#if src.category || domain || src.pages?.length}
              <span class="block text-[10px] text-muted-foreground/80 mt-1 truncate">
                {[src.pages?.length ? `p. ${formatPages(src.pages)}` : '', src.category, domain].filter(Boolean).join(' · ')}
              </span>
            {/if}
          </span>
        </svelte:element>
      </li>
    {/each}
  </ol>
</details>

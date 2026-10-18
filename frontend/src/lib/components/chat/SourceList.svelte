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

  <!-- One compact line per source: number, title, where it is. The snippet shows on hover. -->
  <ol class="mt-2 max-w-xl rounded-lg border bg-card divide-y overflow-hidden">
    {#each sources as src, i}
      {@const hasUrl = src.source_url?.startsWith('http')}
      {@const domain = hostname(src.source_url)}
      <li>
        <svelte:element
          this={hasUrl ? 'a' : 'div'}
          href={hasUrl ? src.source_url : undefined}
          target={hasUrl ? '_blank' : undefined}
          rel={hasUrl ? 'noopener noreferrer' : undefined}
          title={src.snippet || undefined}
          class="group/src flex items-center gap-2.5 px-3 py-2 text-xs
                 {hasUrl ? 'hover:bg-primary/5' : ''} transition-colors"
        >
          <span class="shrink-0 w-5 h-5 rounded-md bg-primary/10 text-primary text-[10px] font-bold flex items-center justify-center">
            {i + 1}
          </span>
          <span class="flex-1 min-w-0 truncate font-medium {hasUrl ? 'group-hover/src:text-primary' : ''}">
            {src.title || domain || 'Document'}
          </span>
          {#if src.pages?.length}
            <span class="shrink-0 text-[11px] text-muted-foreground">p. {formatPages(src.pages)}</span>
          {/if}
          {#if domain}
            <span class="hidden sm:block shrink-0 max-w-[35%] truncate text-[11px] text-muted-foreground">{domain}</span>
          {/if}
          {#if hasUrl}
            <ExternalLink size={12} class="shrink-0 text-muted-foreground group-hover/src:text-primary" />
          {/if}
        </svelte:element>
      </li>
    {/each}
  </ol>
</details>

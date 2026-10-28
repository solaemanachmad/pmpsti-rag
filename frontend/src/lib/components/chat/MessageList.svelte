<script lang="ts">
  import { renderMarkdown } from '$lib/markdown';
  import type { SourceRef } from '$lib/api/client';
  import CitationPopover from './CitationPopover.svelte';
  import Alert from '../ui/Alert.svelte';
  import { GraduationCap } from 'lucide-svelte';

  interface Message {
    role: 'user' | 'assistant';
    content: string;
    sources?: SourceRef[];
  }

  export let messages: Message[];
  export let streamingText = '';
  export let isStreaming = false;
  export let error = '';

  let listEl: HTMLElement | null = null;
  /** sources of the message that contains a citation mark */
  const sourcesFor = (mark: HTMLElement) =>
    messages[Number(mark.closest<HTMLElement>('[data-msg]')?.dataset.msg)]?.sources;

  const avatar = 'w-9 h-9 rounded-full bg-primary text-primary-foreground flex items-center justify-center shrink-0 shadow-sm';
</script>

<div class="space-y-7" bind:this={listEl}>
  {#each messages as msg, i}
    {#if msg.role === 'user'}
      <div class="flex justify-end">
        <div class="bg-primary text-primary-foreground px-4 py-2.5 rounded-2xl rounded-br-md max-w-[85%] sm:max-w-[75%]
                    text-sm leading-relaxed whitespace-pre-wrap break-words shadow-sm">
          {msg.content}
        </div>
      </div>
    {:else}
      <div class="flex gap-3">
        <div class={avatar} aria-hidden="true"><GraduationCap size={20} /></div>
        <div class="flex-1 min-w-0 pt-0.5">
          <!-- citations [n] link to their source; hover or tap shows a preview -->
          <div class="prose-chat text-[15px] leading-relaxed" data-msg={i}>{@html renderMarkdown(msg.content, msg.sources)}</div>
        </div>
      </div>
    {/if}
  {/each}

  {#if isStreaming || streamingText}
    <div class="flex gap-3">
      <div class={avatar} aria-hidden="true"><GraduationCap size={20} /></div>
      <div class="flex-1 min-w-0 pt-0.5">
        {#if streamingText}
          <div class="prose-chat text-[15px] leading-relaxed">
            {@html renderMarkdown(streamingText)}
          </div>
          {#if isStreaming}<span class="cursor-blink text-muted-foreground"></span>{/if}
        {:else}
          <div class="flex items-center gap-2 text-sm text-muted-foreground h-7">
            <span class="flex gap-1 text-primary"><span class="typing-dot"></span><span class="typing-dot"></span><span class="typing-dot"></span></span>
            Searching the documents…
          </div>
        {/if}
      </div>
    </div>
  {/if}

  {#if error}
    <Alert>{error}</Alert>
  {/if}
</div>

{#if listEl}<CitationPopover root={listEl} {sourcesFor} />{/if}

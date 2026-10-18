<script lang="ts">
  import { renderMarkdown } from '$lib/markdown';
  import type { SourceRef } from '$lib/api/client';
  import SourceList from './SourceList.svelte';
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

  const avatar = 'w-9 h-9 rounded-full bg-primary text-primary-foreground flex items-center justify-center shrink-0 shadow-sm';
</script>

<div class="space-y-7">
  {#each messages as msg}
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
          <div class="prose-chat text-[15px] leading-relaxed">{@html renderMarkdown(msg.content)}</div>
          {#if msg.sources?.length}
            <SourceList sources={msg.sources} />
          {/if}
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


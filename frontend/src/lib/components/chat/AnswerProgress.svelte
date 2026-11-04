<script lang="ts">
  // Steps shown while an answer is on its way: searching → reading the sources found → writing.
  // Driven by the backend's status lines; a backend that sends none shows "Searching" until text arrives.
  import { onDestroy, onMount } from 'svelte';
  import { fly } from 'svelte/transition';
  import { Check, Loader2, Search, BookOpen, PenLine } from 'lucide-svelte';
  import type { AnswerStatus } from '$lib/api/client';

  export let status: AnswerStatus | null = null;

  let started = Date.now();
  let now = Date.now();
  let timer: ReturnType<typeof setInterval>;
  onMount(() => { timer = setInterval(() => (now = Date.now()), 500); });
  onDestroy(() => clearInterval(timer));

  $: stage = status?.stage ?? 'searching';
  $: sources = status?.sources ?? [];
  $: seconds = Math.floor((now - started) / 1000);
  $: order = ['searching', 'reading', 'writing'].indexOf(stage);
  $: shownSources = sources.slice(0, 3);
</script>

<ol class="space-y-2 text-sm" aria-live="polite" aria-label="Answer progress">
  <li class="flex items-center gap-2.5">
    <span class="step-icon {order > 0 ? 'done' : 'active'}">
      {#if order > 0}<Check size={12} strokeWidth={3} />{:else}<Search size={12} />{/if}
    </span>
    <span class={order > 0 ? 'text-muted-foreground' : 'text-foreground font-medium'}>
      {order > 0 ? 'Searched the documents' : 'Searching the documents'}
      {#if order === 0}<span class="text-muted-foreground font-normal tabular-nums"> · {seconds} s</span>{/if}
    </span>
    {#if order === 0}<Loader2 size={13} class="animate-spin text-primary" />{/if}
  </li>
  {#if order === 0 && seconds >= 10}
    <li class="pl-7 text-xs text-muted-foreground" in:fly={{ y: -4, duration: 200 }}>
      The server may be waking up. This can take up to a minute.
    </li>
  {/if}

  {#if order >= 1}
    <li class="flex items-start gap-2.5" in:fly={{ y: -6, duration: 220 }}>
      <span class="step-icon {order > 1 ? 'done' : sources.length ? 'active' : 'idle'}">
        {#if order > 1}<Check size={12} strokeWidth={3} />{:else}<BookOpen size={12} />{/if}
      </span>
      <div class="min-w-0 flex-1">
        <span class={order > 1 ? 'text-muted-foreground' : 'text-foreground font-medium'}>
          {#if sources.length}
            {order > 1 ? 'Read' : 'Reading'} {sources.length} {sources.length === 1 ? 'source' : 'sources'}
          {:else}
            No matching documents found
          {/if}
        </span>
        {#if shownSources.length}
          <div class="flex flex-wrap gap-1.5 mt-1.5">
            {#each shownSources as s, i}
              <span class="inline-flex items-center max-w-[16rem] px-2 py-0.5 rounded-full border bg-card text-[11px] text-muted-foreground"
                in:fly={{ y: 4, duration: 200, delay: 80 * i }} title={s.site ? `${s.title} · ${s.site}` : s.title}>
                <span class="truncate">{s.title || s.site || 'Document'}</span>
              </span>
            {/each}
            {#if sources.length > shownSources.length}
              <span class="px-2 py-0.5 text-[11px] text-muted-foreground">+{sources.length - shownSources.length} more</span>
            {/if}
          </div>
        {/if}
      </div>
    </li>
  {/if}

  {#if order >= 2}
    <li class="flex items-center gap-2.5" in:fly={{ y: -6, duration: 220 }}>
      <span class="step-icon active"><PenLine size={12} /></span>
      <span class="text-foreground font-medium">Writing the answer</span>
      <span class="flex gap-1 text-primary" aria-hidden="true"><span class="typing-dot"></span><span class="typing-dot"></span><span class="typing-dot"></span></span>
    </li>
  {/if}
</ol>

<style>
  .step-icon {
    width: 1.25rem; height: 1.25rem; flex-shrink: 0;
    display: inline-flex; align-items: center; justify-content: center;
    border-radius: 9999px;
  }
  .step-icon.active { background: hsl(var(--primary) / 0.12); color: hsl(var(--primary)); }
  .step-icon.done   { background: hsl(var(--primary)); color: hsl(var(--primary-foreground)); }
  .step-icon.idle   { background: hsl(var(--muted)); color: hsl(var(--muted-foreground)); }
</style>

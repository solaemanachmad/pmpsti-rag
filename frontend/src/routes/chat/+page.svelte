<script lang="ts">
  import { onDestroy, onMount, tick } from 'svelte';
  import { browser } from '$app/environment';
  import { page } from '$app/stores';
  import { goto } from '$app/navigation';
  import { chat, askStream, faq } from '$lib/api/client';
  import type { ChatMessage } from '$lib/api/client';
  import { currentUser } from '$lib/stores/auth';
  import { activeSessionId, refreshSessions } from '$lib/stores/chat';
  import { errorMessage } from '$lib/format';
  import MessageList from '$lib/components/chat/MessageList.svelte';
  import Composer from '$lib/components/chat/Composer.svelte';
  import { Sparkles, GraduationCap } from 'lucide-svelte';

  let messages: ChatMessage[] = [];
  let streamingText = '';
  let isStreaming = false;
  let query = '';
  let error = '';
  let loadingSession = false;
  let suggestions: string[] = [];

  let scroller: HTMLDivElement;
  let composer: Composer;
  let cancelStream: (() => void) | null = null;

  onMount(() => {
    faq.list().then(qs => (suggestions = qs.slice(0, 4))).catch(() => {});
  });

  // React to ?s= changes, including clicks in the sidebar history.
  $: if (browser) syncWithUrl($page.url.searchParams.get('s'));

  async function syncWithUrl(sid: string | null) {
    if (sid === ($activeSessionId ?? null)) return;
    stopStreaming();
    error = '';
    if (!sid) {
      activeSessionId.set(undefined);
      messages = [];
      tick().then(() => composer?.focus());
      return;
    }
    activeSessionId.set(sid);
    loadingSession = true;
    try {
      const s = await chat.getSession(sid);
      if ($activeSessionId !== sid) return;
      messages = s.messages;
      await scrollBottom();
    } catch (e) {
      error = 'Could not load this conversation. ' + errorMessage(e);
      messages = [];
    } finally {
      loadingSession = false;
    }
  }

  function stopStreaming() {
    cancelStream?.();
    cancelStream = null;
    isStreaming = false;
    streamingText = '';
  }

  onDestroy(() => {
    stopStreaming();
    activeSessionId.set(undefined);
  });

  function send(text: string) {
    if (isStreaming) return;
    query = '';
    error = '';
    messages = [...messages, { role: 'user', content: text, created_at: new Date().toISOString() }];
    scrollBottom();

    isStreaming = true;
    streamingText = '';
    const startedIn = $activeSessionId;

    cancelStream = askStream(
      text,
      startedIn,
      (chunk) => { streamingText += chunk; scrollBottom(); },
      async (sources, sid) => {
        messages = [...messages, { role: 'assistant', content: streamingText, created_at: new Date().toISOString(), sources }];
        streamingText = '';
        isStreaming = false;
        cancelStream = null;
        if (sid && sid !== startedIn) {
          activeSessionId.set(sid);
          goto(`/chat?s=${sid}`, { replaceState: true, noScroll: true, keepFocus: true });
        }
        refreshSessions();
        scrollBottom();
      },
      (err) => {
        error = err;
        isStreaming = false;
        streamingText = '';
        cancelStream = null;
      }
    );
  }

  async function scrollBottom() {
    await tick();
    scroller?.scrollTo({ top: scroller.scrollHeight });
  }

  $: firstName = ($currentUser?.display_name || '').split(' ')[0];
  $: isEmpty = messages.length === 0 && !isStreaming && !loadingSession;
</script>

<svelte:head><title>Chat · DTETI Answers</title></svelte:head>

<div class="flex flex-col h-full">
  <div bind:this={scroller} class="flex-1 overflow-y-auto">
    <div class="max-w-3xl mx-auto px-4 sm:px-6 py-6 sm:py-10 {isEmpty ? 'h-full' : ''}">
      {#if loadingSession}
        <div class="space-y-6 animate-pulse" aria-label="Loading conversation">
          <div class="ml-auto h-10 w-2/3 rounded-2xl bg-muted"></div>
          <div class="space-y-2"><div class="h-3 w-full rounded bg-muted"></div><div class="h-3 w-5/6 rounded bg-muted"></div><div class="h-3 w-3/5 rounded bg-muted"></div></div>
        </div>
      {:else if isEmpty}
        <div class="h-full flex flex-col items-center justify-center text-center">
          <div class="w-14 h-14 rounded-2xl bg-primary text-primary-foreground flex items-center justify-center mb-5 shadow-soft" aria-hidden="true">
            <GraduationCap size={30} />
          </div>
          <h1 class="text-2xl sm:text-3xl font-extrabold tracking-tight">
            {firstName ? `Hi ${firstName}, what` : 'What'} would you like to know?
          </h1>
          <p class="text-sm text-muted-foreground mt-2 max-w-md">
            Answers are drawn from official DTETI documents, with sources you can check.
          </p>
          {#if error}<p class="text-sm text-destructive mt-4">{error}</p>{/if}
          {#if suggestions.length}
            <div class="grid sm:grid-cols-2 gap-2.5 mt-8 w-full max-w-2xl">
              {#each suggestions as s}
                <button on:click={() => send(s)}
                  class="group text-left p-4 rounded-xl border bg-card hover:border-primary/40 hover:bg-primary/5 transition-colors shadow-soft">
                  <Sparkles size={14} class="text-accent mb-2" />
                  <span class="text-sm leading-snug line-clamp-2">{s}</span>
                </button>
              {/each}
            </div>
          {/if}
        </div>
      {:else}
        <MessageList {messages} {streamingText} {isStreaming} {error} />
      {/if}
    </div>
  </div>

  <div class="shrink-0 px-4 sm:px-6 pt-2 pb-[max(1rem,env(safe-area-inset-bottom))] bg-gradient-to-t from-background via-background to-background/0">
    <div class="max-w-3xl mx-auto">
      <Composer bind:this={composer} bind:value={query} disabled={isStreaming} placeholder="Ask about PMPSTI academic policies…" on:submit={(e) => send(e.detail)} />
      <p class="hidden sm:block text-[11px] text-muted-foreground text-center mt-2">
        Enter to send · Shift + Enter for a new line · Always double-check important details with the faculty
      </p>
    </div>
  </div>
</div>

<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { goto } from '$app/navigation';
  import { isLoggedIn } from '$lib/stores/auth';
  import { askPublic, faq } from '$lib/api/client';
  import type { GuestAskResponse, SourceRef } from '$lib/api/client';
  import { renderMarkdown } from '$lib/markdown';
  import Logo from '$lib/components/Logo.svelte';
  import ThemeToggle from '$lib/components/ThemeToggle.svelte';
  import MessageList from '$lib/components/chat/MessageList.svelte';
  import Composer from '$lib/components/chat/Composer.svelte';
  import { ChevronDown, Loader2, BookOpenCheck, Clock3, Quote, ArrowRight, GraduationCap } from 'lucide-svelte';

  const GUEST_QUOTA = 5;
  const FAQ_INITIAL = 4;

  // ── Guest quota (resets daily, mirrored from the server response) ──
  const today = () => new Date().toISOString().slice(0, 10);

  function getGuestToken(): string {
    try {
      let t = localStorage.getItem('guest_token');
      if (!t) {
        t = 'guest_' + Math.random().toString(36).slice(2) + Date.now().toString(36);
        localStorage.setItem('guest_token', t);
      }
      return t;
    } catch { return 'guest_' + Math.random().toString(36).slice(2); }
  }

  function getQuestionsUsed(): number {
    try {
      if (localStorage.getItem('guest_questions_day') !== today()) {
        localStorage.setItem('guest_questions_used', '0');
        localStorage.setItem('guest_questions_day', today());
        return 0;
      }
      return parseInt(localStorage.getItem('guest_questions_used') ?? '0', 10);
    } catch { return 0; }
  }

  function setQuestionsUsed(n: number) {
    try {
      localStorage.setItem('guest_questions_used', String(n));
      localStorage.setItem('guest_questions_day', today());
    } catch { /* storage blocked */ }
  }

  interface Message { role: 'user' | 'assistant'; content: string; sources?: SourceRef[] }

  let messages: Message[] = [];
  let query = '';
  let streamingText = '';
  let isStreaming = false;
  let error = '';
  let quotaExceeded = false;
  let questionsLeft = GUEST_QUOTA;
  let scroller: HTMLDivElement;

  const fallbackFaq = [
    'What are the graduation requirements for the PMPSTI master’s program?',
    'How do I pay the tuition fee (UKT)?',
    'How do I apply for an academic leave?'
  ];
  let faqQuestions: string[] = [];
  let faqOpen: number | null = null;
  let faqAnswers: Record<number, string> = {};
  let faqLoading: Record<number, boolean> = {};
  let faqExpanded = false;

  onMount(async () => {
    if ($isLoggedIn) { goto('/chat'); return; }
    questionsLeft = Math.max(GUEST_QUOTA - getQuestionsUsed(), 0);
    quotaExceeded = questionsLeft === 0;
    try {
      const qs = await faq.list();
      faqQuestions = qs.length ? qs : fallbackFaq;
    } catch {
      faqQuestions = fallbackFaq;
    }
  });

  function toggleFaq(idx: number) {
    if (faqOpen === idx) { faqOpen = null; return; }
    faqOpen = idx;
    if (faqAnswers[idx] !== undefined) return;
    faqLoading = { ...faqLoading, [idx]: true };
    let ans = '';
    askPublic(
      faqQuestions[idx], getGuestToken(),
      (chunk) => { ans += chunk; faqAnswers = { ...faqAnswers, [idx]: ans }; },
      () => { faqLoading = { ...faqLoading, [idx]: false }; },
      () => { faqLoading = { ...faqLoading, [idx]: false }; faqAnswers = { ...faqAnswers, [idx]: 'Could not load the answer. Please try again later.' }; }
    );
  }

  async function scrollBottom() {
    await tick();
    scroller?.scrollTo({ top: scroller.scrollHeight });
  }

  function send(text: string) {
    if (!text || isStreaming || quotaExceeded) return;
    query = '';
    error = '';
    messages = [...messages, { role: 'user', content: text }];
    scrollBottom();
    isStreaming = true;
    streamingText = '';

    askPublic(
      text, getGuestToken(),
      (chunk) => { streamingText += chunk; scrollBottom(); },
      (res: GuestAskResponse) => {
        messages = [...messages, { role: 'assistant', content: streamingText, sources: res.sources }];
        streamingText = '';
        isStreaming = false;
        questionsLeft = res.questions_left;
        setQuestionsUsed(res.questions_used);
        if (questionsLeft <= 0) quotaExceeded = true;
        scrollBottom();
      },
      (err: string) => {
        if (err === '__quota_exceeded__') { quotaExceeded = true; questionsLeft = 0; }
        else error = err;
        isStreaming = false;
        streamingText = '';
      }
    );
  }

  $: chatting = messages.length > 0 || isStreaming;
  $: visibleFaq = faqExpanded ? faqQuestions : faqQuestions.slice(0, FAQ_INITIAL);

  const features = [
    { icon: BookOpenCheck, title: 'Official documents', text: 'Answers come only from published DTETI regulations and guides.' },
    { icon: Quote,         title: 'Cited sources',      text: 'Every answer links back to the documents it used.' },
    { icon: Clock3,        title: 'Available 24/7',     text: 'Get answers instantly, any time — no queue.' }
  ];
</script>

<svelte:head>
  <title>DTETI Answers</title>
  <meta name="description" content="Ask questions about DTETI at Universitas Gadjah Mada. AI answers grounded in official academic documents, instantly, 24/7." />
</svelte:head>

<div class="h-dvh flex flex-col bg-background overflow-hidden">
  <!-- Header -->
  <header class="shrink-0 h-16 border-b bg-card/70 backdrop-blur-md z-30">
    <div class="h-full max-w-6xl mx-auto px-4 sm:px-6 flex items-center justify-between gap-3">
      <a href="/" class="min-w-0" on:click={() => { messages = []; error = ''; }}>
        <Logo />
      </a>
      <nav class="flex items-center gap-1.5 shrink-0">
        <ThemeToggle />
        <a href="/login" class="btn-ghost h-9 px-3">Sign in</a>
        <a href="/register" class="btn-gold h-9 px-3.5 hidden sm:inline-flex">Create account</a>
      </nav>
    </div>
  </header>

  <div bind:this={scroller} class="flex-1 min-h-0 overflow-y-auto">
    {#if !chatting}
      <!-- Hero -->
      <section class="relative overflow-hidden">
        <div class="pointer-events-none absolute inset-0 -z-0" aria-hidden="true">
          <div class="absolute -top-40 left-1/2 -translate-x-1/2 w-[900px] h-[500px] rounded-full bg-primary/10 blur-3xl"></div>
          <div class="absolute top-10 right-[10%] w-72 h-72 rounded-full bg-accent/15 blur-3xl"></div>
          <div class="absolute inset-0 opacity-[0.35] dark:opacity-[0.15]
                      [background-image:radial-gradient(hsl(var(--border))_1px,transparent_1px)] [background-size:22px_22px]
                      [mask-image:linear-gradient(to_bottom,black,transparent_75%)]"></div>
        </div>

        <div class="relative max-w-3xl mx-auto px-4 sm:px-6 pt-12 sm:pt-20 pb-10 text-center">
          <span class="inline-flex items-center gap-1.5 text-[11px] font-bold uppercase tracking-[0.14em]
                       text-primary bg-primary/10 border border-primary/15 px-3 py-1 rounded-full">
            <GraduationCap size={13} /> DTETI Academic Assistant
          </span>
          <h1 class="mt-5 text-4xl sm:text-5xl lg:text-6xl font-extrabold tracking-tight leading-[1.05]">
            Academic questions,<br class="hidden sm:block" />
            <span class="bg-gradient-to-r from-primary to-warning dark:to-accent bg-clip-text text-transparent">answered from the source.</span>
          </h1>
          <p class="mt-5 text-base sm:text-lg text-muted-foreground max-w-xl mx-auto">
            Ask anything about DTETI rules, curriculum and procedures. Answers come straight from official UGM documents.
          </p>

          <div class="mt-8 text-left">
            {#if quotaExceeded}
              <div class="card p-5 text-center">
                <p class="font-bold">You’ve used today’s {GUEST_QUOTA} free questions</p>
                <p class="text-sm text-muted-foreground mt-1">Create a free account with your <strong>@mail.ugm.ac.id</strong> email for unlimited questions and chat history.</p>
                <div class="flex flex-col sm:flex-row gap-2 justify-center mt-4">
                  <a href="/register" class="btn-gold">Create free account</a>
                  <a href="/login" class="btn-outline">I already have an account</a>
                </div>
              </div>
            {:else}
              <Composer bind:value={query} placeholder="e.g. How many credits do I need to graduate?" on:submit={(e) => send(e.detail)} />
              <p class="text-xs text-muted-foreground text-center mt-2.5">
                {questionsLeft} of {GUEST_QUOTA} free questions left today ·
                <a href="/register" class="font-semibold text-primary hover:underline">Sign up</a> for unlimited access
              </p>
            {/if}
          </div>
        </div>
      </section>

      <!-- FAQ -->
      {#if faqQuestions.length}
        <section class="max-w-3xl mx-auto px-4 sm:px-6 pb-12">
          <h2 class="text-xs font-bold uppercase tracking-[0.14em] text-muted-foreground mb-3">Popular questions</h2>
          <div class="card divide-y overflow-hidden">
            {#each visibleFaq as q, idx}
              <div>
                <button on:click={() => toggleFaq(idx)} aria-expanded={faqOpen === idx}
                  class="w-full flex items-center justify-between gap-3 px-4 sm:px-5 py-4 text-left hover:bg-muted/40 transition-colors">
                  <span class="text-sm font-medium leading-snug">{q}</span>
                  <ChevronDown size={16} class="shrink-0 text-muted-foreground transition-transform {faqOpen === idx ? 'rotate-180' : ''}" />
                </button>
                {#if faqOpen === idx}
                  <div class="px-4 sm:px-5 pb-4">
                    {#if faqAnswers[idx]}
                      <div class="prose-chat text-sm text-foreground/90 max-h-64 overflow-y-auto">{@html renderMarkdown(faqAnswers[idx])}</div>
                      {#if !faqLoading[idx] && !quotaExceeded}
                        <button on:click={() => send(q)} class="mt-3 inline-flex items-center gap-1 text-xs font-semibold text-primary hover:underline">
                          Ask a follow-up <ArrowRight size={12} />
                        </button>
                      {/if}
                    {:else}
                      <p class="flex items-center gap-2 text-xs text-muted-foreground"><Loader2 size={13} class="animate-spin" /> Finding the answer…</p>
                    {/if}
                  </div>
                {/if}
              </div>
            {/each}
          </div>
          {#if faqQuestions.length > FAQ_INITIAL}
            <button on:click={() => { faqExpanded = !faqExpanded; if (!faqExpanded) faqOpen = null; }}
              class="mt-3 mx-auto flex items-center gap-1.5 text-xs font-semibold text-muted-foreground hover:text-foreground">
              <ChevronDown size={13} class="transition-transform {faqExpanded ? 'rotate-180' : ''}" />
              {faqExpanded ? 'Show fewer' : `Show ${faqQuestions.length - FAQ_INITIAL} more questions`}
            </button>
          {/if}
        </section>
      {/if}

      <!-- Features -->
      <section class="max-w-5xl mx-auto px-4 sm:px-6 pb-16">
        <div class="grid sm:grid-cols-3 gap-3">
          {#each features as f}
            <div class="card p-5">
              <div class="w-9 h-9 rounded-lg bg-accent/15 text-warning flex items-center justify-center mb-3">
                <svelte:component this={f.icon} size={18} />
              </div>
              <p class="font-bold text-sm">{f.title}</p>
              <p class="text-sm text-muted-foreground mt-1">{f.text}</p>
            </div>
          {/each}
        </div>
      </section>

      <footer class="border-t bg-card/70 backdrop-blur-md py-6 text-center text-xs text-muted-foreground px-4">
        © {new Date().getFullYear()} Department of Electrical Engineering & Information Technology · Universitas Gadjah Mada
      </footer>
    {:else}
      <div class="max-w-3xl mx-auto px-4 sm:px-6 py-6 sm:py-10">
        <MessageList {messages} {streamingText} {isStreaming} {error} />
      </div>
    {/if}
  </div>

  {#if chatting}
    <div class="shrink-0 px-4 sm:px-6 pt-2 pb-[max(1rem,env(safe-area-inset-bottom))]">
      <div class="max-w-3xl mx-auto">
        {#if quotaExceeded}
          <div class="card p-4 flex flex-col sm:flex-row items-center gap-3 sm:justify-between text-center sm:text-left">
            <div>
              <p class="text-sm font-bold">Daily free limit reached</p>
              <p class="text-xs text-muted-foreground">Sign up with your @mail.ugm.ac.id email for unlimited questions.</p>
            </div>
            <div class="flex gap-2 shrink-0">
              <a href="/login" class="btn-outline h-9">Sign in</a>
              <a href="/register" class="btn-gold h-9">Create account</a>
            </div>
          </div>
        {:else}
          <Composer bind:value={query} disabled={isStreaming} placeholder="Ask a follow-up…" on:submit={(e) => send(e.detail)} />
          <p class="text-[11px] text-muted-foreground text-center mt-2">
            {questionsLeft} free {questionsLeft === 1 ? 'question' : 'questions'} left ·
            <a href="/register" class="font-semibold text-primary hover:underline">Sign up</a> for unlimited access ·
            Not a substitute for official academic advice
          </p>
        {/if}
      </div>
    </div>
  {/if}
</div>

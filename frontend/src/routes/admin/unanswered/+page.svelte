<script lang="ts">
  import { onMount } from 'svelte';
  import { admin } from '$lib/api/client';
  import type { UnansweredQuestion } from '$lib/api/client';
  import { errorMessage, formatDateTime } from '$lib/format';
  import PageHeader from '$lib/components/ui/PageHeader.svelte';
  import Alert from '$lib/components/ui/Alert.svelte';
  import LoadingState from '$lib/components/ui/LoadingState.svelte';
  import EmptyState from '$lib/components/ui/EmptyState.svelte';
  import { RefreshCw, Check, FolderSync, Copy, MessageCircleQuestion } from 'lucide-svelte';

  let questions: UnansweredQuestion[] = [];
  let days = 30;
  let includeNoSources = false;
  let loading = false;
  let error = '';
  let notice = '';
  let busy: Record<string, boolean> = {};

  onMount(load);

  async function load() {
    loading = true; error = '';
    try {
      questions = await admin.unanswered(days, includeNoSources);
    } catch (e) {
      error = errorMessage(e);
    } finally {
      loading = false;
    }
  }

  async function resolve(q: UnansweredQuestion) {
    busy = { ...busy, [q.query]: true };
    try {
      await admin.resolveUnanswered(q.ids);
      questions = questions.filter(x => x !== q);
      notice = `Marked “${q.query}” as handled.`;
    } catch (e) {
      error = errorMessage(e);
    } finally {
      busy = { ...busy, [q.query]: false };
    }
  }

  async function copy(q: UnansweredQuestion) {
    try { await navigator.clipboard.writeText(q.query); notice = 'Question copied — paste it in Chat to test after adding a source.'; }
    catch { /* clipboard blocked */ }
  }

  $: totalAsks = questions.reduce((n, q) => n + q.times, 0);
</script>

<svelte:head><title>Unanswered · Admin</title></svelte:head>

<PageHeader title="Unanswered questions"
  description="Questions the documents could not answer. Add a source that covers them, then mark them handled.">
  <button on:click={load} class="btn-outline" disabled={loading}>
    <RefreshCw size={15} class={loading ? 'animate-spin' : ''} /> Refresh
  </button>
</PageHeader>

<div class="flex flex-col sm:flex-row sm:items-center gap-3 mb-4 text-sm">
  <label class="flex items-center gap-2">
    <span class="text-muted-foreground">Period</span>
    <select bind:value={days} on:change={load} class="input !w-auto !py-1.5">
      <option value={7}>Last 7 days</option>
      <option value={30}>Last 30 days</option>
      <option value={90}>Last 90 days</option>
      <option value={365}>Last year</option>
    </select>
  </label>
  <label class="inline-flex items-center gap-2 cursor-pointer select-none">
    <input type="checkbox" bind:checked={includeNoSources} on:change={load} class="w-4 h-4 rounded accent-[hsl(var(--primary))]" />
    <span>Also show questions with no sources found <span class="text-muted-foreground">(includes greetings)</span></span>
  </label>
</div>

{#if error}<div class="mb-4"><Alert>{error}</Alert></div>{/if}
{#if notice}<div class="mb-4"><Alert variant="success">{notice}</Alert></div>{/if}

{#if loading && questions.length === 0}
  <LoadingState />
{:else if questions.length === 0}
  <div class="card">
    <EmptyState icon={MessageCircleQuestion} title="Nothing unanswered"
      description="Every question in this period was answered from the documents, or has been marked handled. Questions asked before this page existed are not classified." />
  </div>
{:else}
  <p class="text-xs text-muted-foreground mb-2 px-1">{questions.length} {questions.length === 1 ? 'question' : 'questions'} · asked {totalAsks}× in total · most asked first</p>
  <ul class="card divide-y overflow-hidden {loading ? 'opacity-60' : ''} transition-opacity">
    {#each questions as q (q.query)}
      <li class="p-4 flex flex-col sm:flex-row sm:items-center gap-3">
        <div class="flex-1 min-w-0">
          <p class="text-sm font-medium leading-relaxed break-words">{q.query}</p>
          <p class="flex flex-wrap items-center gap-1.5 mt-1.5 text-xs text-muted-foreground">
            <span class={q.times > 1 ? 'badge-warning' : 'badge-neutral'}>asked {q.times}×</span>
            {#if q.not_found < q.times}<span class="badge-neutral" title="No relevant document was found">{q.times - q.not_found}× no sources</span>{/if}
            <span>last {formatDateTime(q.last_asked)}</span>
            {#if q.times > 1}<span>· first {formatDateTime(q.first_asked)}</span>{/if}
          </p>
        </div>
        <div class="flex items-center gap-1 shrink-0">
          <button class="icon-btn" on:click={() => copy(q)} title="Copy question" aria-label="Copy question"><Copy size={15} /></button>
          <a href="/admin/sources" class="btn-ghost btn-sm"><FolderSync size={13} /> Add a source</a>
          <button class="btn-outline btn-sm" on:click={() => resolve(q)} disabled={busy[q.query]}>
            <Check size={13} /> Mark handled
          </button>
        </div>
      </li>
    {/each}
  </ul>
{/if}

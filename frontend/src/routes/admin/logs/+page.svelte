<script lang="ts">
  // Every question asked, newest first — to check what students ask, whether it was answered and
  // how long it took. Who asked is not shown (only whether it was a guest).
  import { onMount } from 'svelte';
  import { admin } from '$lib/api/client';
  import type { AdminQueryLog, QueryLogFilter } from '$lib/api/client';
  import { errorMessage, formatDateTime } from '$lib/format';
  import PageHeader from '$lib/components/ui/PageHeader.svelte';
  import Alert from '$lib/components/ui/Alert.svelte';
  import LoadingState from '$lib/components/ui/LoadingState.svelte';
  import EmptyState from '$lib/components/ui/EmptyState.svelte';
  import { RefreshCw, ChevronLeft, ChevronRight, ScrollText, Search } from 'lucide-svelte';

  const PER_PAGE = 50;
  let logs: AdminQueryLog[] = [];
  let total = 0;
  let pageIdx = 0;
  let loading = false;
  let error = '';

  let outcome: QueryLogFilter['outcome'] = '';
  let who: QueryLogFilter['who'] = '';
  let days = 30;
  let search = '';

  const OUTCOMES = [
    { value: '', label: 'All answers' },
    { value: 'answered', label: 'Answered' },
    { value: 'not_found', label: 'Not in the documents' },
    { value: 'no_sources', label: 'No sources found' }
  ];

  async function loadLogs(p: number) {
    loading = true; error = ''; pageIdx = p;
    try {
      const r = await admin.queryLogs(PER_PAGE, p * PER_PAGE, { outcome, who, days, q: search });
      logs = r.logs; total = r.total;
    } catch (e) { error = errorMessage(e); }
    finally { loading = false; }
  }

  // Filters reload from the first page; the search waits until typing stops
  let searchTimer: ReturnType<typeof setTimeout>;
  let ready = false;
  onMount(() => { ready = true; });
  $: if (ready) { outcome; who; days; loadLogs(0); }
  function onSearch() {
    clearTimeout(searchTimer);
    searchTimer = setTimeout(() => loadLogs(0), 350);
  }

  $: totalPages = Math.max(1, Math.ceil(total / PER_PAGE));
  $: filtered = !!outcome || !!who || !!search.trim();

  const outcomeBadge = (o?: string) =>
    o === 'answered' ? ['badge-success', 'answered']
    : o === 'not_found' ? ['badge-warning', 'not in documents']
    : o === 'no_sources' ? ['badge-danger', 'no sources']
    : ['badge-neutral', '—'];
</script>

<svelte:head><title>Query Logs · Admin</title></svelte:head>

<PageHeader title="Query Logs"
  description="{total.toLocaleString('en')} {total === 1 ? 'question' : 'questions'}{days ? ` in the last ${days} days` : ''}. Who asked is not shown. Kept 180 days.">
  <button on:click={() => loadLogs(pageIdx)} class="btn-outline" disabled={loading}>
    <RefreshCw size={15} class={loading ? 'animate-spin' : ''} /> <span class="hidden sm:inline">Refresh</span>
  </button>
</PageHeader>

<div class="flex flex-col lg:flex-row gap-3 mb-4">
  <div class="relative flex-1">
    <Search size={15} class="absolute left-3 top-1/2 -translate-y-1/2 text-muted-foreground" />
    <input bind:value={search} on:input={onSearch} placeholder="Search questions…" class="input pl-9" aria-label="Search questions" />
  </div>
  <div class="grid grid-cols-3 gap-3 lg:flex">
    <select bind:value={outcome} class="input lg:w-48" aria-label="Filter by answer">
      {#each OUTCOMES as o}<option value={o.value}>{o.label}</option>{/each}
    </select>
    <select bind:value={who} class="input lg:w-36" aria-label="Filter by who asked">
      <option value="">Everyone</option>
      <option value="user">Signed in</option>
      <option value="guest">Guests</option>
    </select>
    <select bind:value={days} class="input lg:w-44" aria-label="Time range">
      <option value={1}>Today</option>
      <option value={7}>Last 7 days</option>
      <option value={30}>Last 30 days</option>
      <option value={180}>Last 180 days</option>
      <option value={0}>All time</option>
    </select>
  </div>
</div>

{#if error}<div class="mb-4"><Alert>{error}</Alert></div>{/if}

{#if loading && logs.length === 0}
  <LoadingState />
{:else if logs.length === 0}
  <div class="card"><EmptyState icon={ScrollText} title={filtered ? 'No questions match these filters' : 'No questions asked in this period'} /></div>
{:else}
  <div class="card overflow-hidden {loading ? 'opacity-60' : ''} transition-opacity">
    <table class="data-table hidden md:table">
      <thead>
        <tr><th>Question</th><th>Answer</th><th class="!text-right">Time</th><th class="!text-right">Date</th></tr>
      </thead>
      <tbody>
        {#each logs as log (log.id)}
          {@const [cls, label] = outcomeBadge(log.outcome)}
          <tr>
            <td>
              <p class="leading-relaxed">{log.query_text}</p>
              <span class="flex flex-wrap gap-1.5 mt-1">
                <span class="badge-neutral">{log.guest ? 'guest' : 'signed in'}</span>
                {#if log.similar > 1}<span class="badge-neutral" title="Questions with the same meaning (typos, other wording) in this list">{log.similar}× similar</span>{/if}
                {#if log.truncated}<span class="badge-warning" title="The answer stopped at the max tokens limit (Admin → Settings)">truncated</span>{/if}
              </span>
            </td>
            <td><span class={cls}>{label}</span> <span class="text-xs text-muted-foreground">{log.num_results} {log.num_results === 1 ? 'source' : 'sources'}</span></td>
            <td class="text-right text-xs font-mono text-muted-foreground whitespace-nowrap">{(log.search_time_ms / 1000).toFixed(1)} s</td>
            <td class="text-right text-xs text-muted-foreground whitespace-nowrap">{formatDateTime(log.created_at)}</td>
          </tr>
        {/each}
      </tbody>
    </table>
    <ul class="md:hidden divide-y">
      {#each logs as log (log.id)}
        {@const [cls, label] = outcomeBadge(log.outcome)}
        <li class="p-4">
          <p class="text-sm leading-relaxed">{log.query_text}</p>
          <div class="flex flex-wrap items-center gap-1.5 mt-2 text-xs text-muted-foreground">
            <span class={cls}>{label}</span>
            <span class="badge-neutral">{log.guest ? 'guest' : 'signed in'}</span>
            {#if log.similar > 1}<span class="badge-neutral">{log.similar}× similar</span>{/if}
            {#if log.truncated}<span class="badge-warning">truncated</span>{/if}
            <span>{(log.search_time_ms / 1000).toFixed(1)} s · {formatDateTime(log.created_at)}</span>
          </div>
        </li>
      {/each}
    </ul>
  </div>
{/if}

{#if totalPages > 1}
  <div class="flex items-center justify-center gap-3 mt-5">
    <button disabled={pageIdx === 0 || loading} on:click={() => loadLogs(pageIdx - 1)} class="icon-btn border" aria-label="Previous page"><ChevronLeft size={16} /></button>
    <span class="text-sm text-muted-foreground">Page {pageIdx + 1} of {totalPages}</span>
    <button disabled={pageIdx >= totalPages - 1 || loading} on:click={() => loadLogs(pageIdx + 1)} class="icon-btn border" aria-label="Next page"><ChevronRight size={16} /></button>
  </div>
{/if}

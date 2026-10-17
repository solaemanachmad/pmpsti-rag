<script lang="ts">
  import { onMount } from 'svelte';
  import { admin } from '$lib/api/client';
  import { errorMessage, formatDateTime } from '$lib/format';
  import PageHeader from '$lib/components/ui/PageHeader.svelte';
  import Alert from '$lib/components/ui/Alert.svelte';
  import LoadingState from '$lib/components/ui/LoadingState.svelte';
  import EmptyState from '$lib/components/ui/EmptyState.svelte';
  import { RefreshCw, ChevronLeft, ChevronRight, ScrollText } from 'lucide-svelte';

  interface QueryLog {
    id: number; query_text: string; detected_language: string;
    num_results: number; search_time_ms: number;
    user_id: number | null; session_id: string | null; created_at: string;
  }

  const PER_PAGE = 50;
  let logs: QueryLog[] = [];
  let total = 0;
  let pageIdx = 0;
  let loading = false;
  let error = '';
  let zeroOnly = false;

  onMount(() => loadLogs(0));

  async function loadLogs(p: number) {
    loading = true; error = ''; pageIdx = p;
    try {
      const r = await admin.queryLogs(PER_PAGE, p * PER_PAGE);
      logs = r.logs; total = r.total;
    } catch (e) { error = errorMessage(e); }
    finally { loading = false; }
  }

  $: totalPages = Math.max(1, Math.ceil(total / PER_PAGE));
  $: shown = zeroOnly ? logs.filter(l => l.num_results === 0) : logs;
</script>

<svelte:head><title>Query Logs · Admin</title></svelte:head>

<PageHeader title="Query Logs" description="{total.toLocaleString('en')} queries recorded">
  <button on:click={() => loadLogs(pageIdx)} class="btn-outline" disabled={loading}>
    <RefreshCw size={15} class={loading ? 'animate-spin' : ''} /> Refresh
  </button>
</PageHeader>

<label class="inline-flex items-center gap-2 text-sm mb-4 cursor-pointer select-none">
  <input type="checkbox" bind:checked={zeroOnly} class="w-4 h-4 rounded accent-[hsl(var(--primary))]" />
  Only show queries with no results <span class="text-muted-foreground">(this page)</span>
</label>

{#if error}<div class="mb-4"><Alert>{error}</Alert></div>{/if}

{#if loading && logs.length === 0}
  <LoadingState />
{:else if shown.length === 0}
  <div class="card"><EmptyState icon={ScrollText} title={zeroOnly ? 'No zero-result queries on this page' : 'No queries logged yet'} /></div>
{:else}
  <div class="card overflow-hidden {loading ? 'opacity-60' : ''} transition-opacity">
    <table class="data-table hidden md:table">
      <thead>
        <tr><th>Query</th><th class="!text-right">Results</th><th class="!text-right">Time</th><th class="!text-right">Date</th></tr>
      </thead>
      <tbody>
        {#each shown as log (log.id)}
          <tr>
            <td>
              <p class="leading-relaxed">{log.query_text}</p>
              <span class="text-[10px] uppercase tracking-wider text-muted-foreground">{log.detected_language}</span>
            </td>
            <td class="text-right"><span class={log.num_results === 0 ? 'badge-danger' : 'badge-success'}>{log.num_results}</span></td>
            <td class="text-right text-xs font-mono text-muted-foreground whitespace-nowrap">{log.search_time_ms} ms</td>
            <td class="text-right text-xs text-muted-foreground whitespace-nowrap">{formatDateTime(log.created_at)}</td>
          </tr>
        {/each}
      </tbody>
    </table>
    <ul class="md:hidden divide-y">
      {#each shown as log (log.id)}
        <li class="p-4">
          <p class="text-sm leading-relaxed">{log.query_text}</p>
          <div class="flex flex-wrap items-center gap-1.5 mt-2 text-xs text-muted-foreground">
            <span class={log.num_results === 0 ? 'badge-danger' : 'badge-success'}>{log.num_results} results</span>
            <span class="badge-neutral uppercase">{log.detected_language}</span>
            <span>{log.search_time_ms} ms · {formatDateTime(log.created_at)}</span>
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

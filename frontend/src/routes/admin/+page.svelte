<script lang="ts">
  import { onMount } from 'svelte';
  import { admin } from '$lib/api/client';
  import type { QueryLogStats } from '$lib/api/client';
  import { errorMessage } from '$lib/format';
  import PageHeader from '$lib/components/ui/PageHeader.svelte';
  import Alert from '$lib/components/ui/Alert.svelte';
  import LoadingState from '$lib/components/ui/LoadingState.svelte';
  import { MessagesSquare, Fingerprint, Layers, SearchX, Timer, TrendingUp, CalendarDays, RefreshCw } from 'lucide-svelte';

  let stats: QueryLogStats | null = null;
  let loading = false;
  let error = '';

  onMount(loadStats);

  async function loadStats() {
    loading = true; error = '';
    try { stats = await admin.stats(); }
    catch (e) { error = errorMessage(e); }
    finally { loading = false; }
  }

  $: kpis = stats ? [
    { label: 'Total queries',      value: stats.total_queries.toLocaleString('en'),       icon: MessagesSquare, tone: 'text-primary bg-primary/10' },
    { label: 'Unique queries',     value: stats.unique_queries.toLocaleString('en'),      icon: Fingerprint,    tone: 'text-success bg-success/10' },
    { label: 'Avg. results',       value: stats.avg_results.toFixed(1),                   icon: Layers,         tone: 'text-warning bg-warning/15' },
    { label: 'Zero-result queries',value: stats.zero_result_queries.toLocaleString('en'), icon: SearchX,        tone: 'text-destructive bg-destructive/10' },
    { label: 'Avg. search time',   value: `${stats.avg_search_time_ms.toFixed(0)} ms`,    icon: Timer,          tone: 'text-primary bg-primary/10' }
  ] : [];

  // Oldest → newest so the chart reads left to right.
  $: days = stats ? [...stats.queries_per_day].slice(0, 30).reverse() : [];
  $: maxDay = Math.max(1, ...days.map(d => d[1]));
  $: zeroRate = stats && stats.total_queries ? (stats.zero_result_queries / stats.total_queries) * 100 : 0;
</script>

<svelte:head><title>Overview · Admin</title></svelte:head>

<PageHeader title="Overview" description="How students are using the assistant.">
  <button on:click={loadStats} class="btn-outline" disabled={loading}>
    <RefreshCw size={15} class={loading ? 'animate-spin' : ''} /> Refresh
  </button>
</PageHeader>

{#if loading && !stats}
  <LoadingState label="Loading statistics…" />
{:else if error}
  <Alert>{error}</Alert>
{:else if stats}
  <div class="grid grid-cols-2 lg:grid-cols-5 gap-3 mb-5">
    {#each kpis as k, i}
      <div class="card p-4 {i === kpis.length - 1 ? 'col-span-2 lg:col-span-1' : ''}">
        <div class="flex items-center justify-between gap-2">
          <span class="text-xs font-semibold text-muted-foreground">{k.label}</span>
          <span class="w-7 h-7 rounded-lg flex items-center justify-center {k.tone}"><svelte:component this={k.icon} size={15} /></span>
        </div>
        <div class="text-2xl font-extrabold tracking-tight mt-2">{k.value}</div>
      </div>
    {/each}
  </div>

  <div class="grid lg:grid-cols-5 gap-4">
    <section class="card p-5 lg:col-span-3">
      <div class="flex items-center justify-between mb-4">
        <h2 class="text-sm font-bold flex items-center gap-2"><CalendarDays size={15} class="text-primary" /> Queries per day</h2>
        <span class="text-xs text-muted-foreground">Last {days.length} days</span>
      </div>
      {#if days.length === 0}
        <p class="text-sm text-muted-foreground py-10 text-center">No data yet.</p>
      {:else}
        <div class="flex items-end gap-[3px] h-44" role="img" aria-label="Bar chart of queries per day">
          {#each days as [day, count]}
            <div class="group relative flex-1 h-full flex items-end">
              <div class="w-full rounded-t bg-primary/80 group-hover:bg-accent transition-colors min-h-[2px]"
                   style="height:{(count / maxDay) * 100}%"></div>
              <div class="pointer-events-none absolute bottom-full left-1/2 -translate-x-1/2 mb-1 hidden group-hover:block
                          whitespace-nowrap rounded-md bg-foreground text-background text-[11px] font-medium px-2 py-1 z-10">
                {day}: {count}
              </div>
            </div>
          {/each}
        </div>
        <div class="flex justify-between text-[11px] text-muted-foreground mt-2">
          <span>{days[0][0]}</span><span>{days[days.length - 1][0]}</span>
        </div>
      {/if}
    </section>

    <section class="card p-5 lg:col-span-2">
      <h2 class="text-sm font-bold flex items-center gap-2 mb-4"><TrendingUp size={15} class="text-primary" /> Top questions</h2>
      {#if stats.top_queries.length === 0}
        <p class="text-sm text-muted-foreground py-10 text-center">No data yet.</p>
      {:else}
        <ol class="space-y-2.5">
          {#each stats.top_queries.slice(0, 8) as [q, count], i}
            <li class="flex items-start gap-3 text-sm">
              <span class="w-5 text-xs font-bold text-muted-foreground pt-0.5">{i + 1}</span>
              <span class="flex-1 min-w-0 leading-snug line-clamp-2">{q}</span>
              <span class="badge-primary shrink-0">{count}×</span>
            </li>
          {/each}
        </ol>
      {/if}
    </section>
  </div>

  {#if stats.total_queries > 0}
    <p class="text-xs text-muted-foreground mt-4">
      {zeroRate.toFixed(1)}% of queries returned no documents — check
      <a href="/admin/logs" class="font-semibold text-primary hover:underline">Query Logs</a>
      for gaps worth covering in <a href="/admin/documents" class="font-semibold text-primary hover:underline">Documents</a>.
    </p>
  {/if}
{/if}

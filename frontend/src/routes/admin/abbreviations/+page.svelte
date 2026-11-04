<script lang="ts">
  // Abbreviations students type ("sempro", "TA"): candidates found in the questions, and the list
  // used to explain them when searching (Settings → Expand abbreviations).
  import { onMount } from 'svelte';
  import { abbreviations } from '$lib/api/client';
  import type { AbbreviationCandidate, AbbreviationEntry } from '$lib/api/client';
  import { errorMessage, formatDate } from '$lib/format';
  import PageHeader from '$lib/components/ui/PageHeader.svelte';
  import Alert from '$lib/components/ui/Alert.svelte';
  import LoadingState from '$lib/components/ui/LoadingState.svelte';
  import EmptyState from '$lib/components/ui/EmptyState.svelte';
  import { RefreshCw, BookA, Check, EyeOff, Trash2, Pencil, Plus, ChevronDown } from 'lucide-svelte';

  let entries: AbbreviationEntry[] = [];
  let builtin: { short: string; expansion: string }[] = [];
  let expandEnabled = false;
  let candidates: AbbreviationCandidate[] = [];
  let days = 90;
  let loading = false;
  let error = '';
  let notice = '';
  let busy: Record<string, boolean> = {};
  /** expansion typed for a candidate or an edited entry, by short form */
  let draft: Record<string, string> = {};
  let editing: string | null = null;
  let newShort = '';
  let newExpansion = '';
  let showBuiltin = false;

  onMount(load);

  async function load() {
    loading = true; error = '';
    try {
      const [l, c] = await Promise.all([abbreviations.list(), abbreviations.candidates(days)]);
      entries = l.entries; builtin = l.builtin; expandEnabled = l.expand_enabled; candidates = c;
    } catch (e) { error = errorMessage(e); }
    finally { loading = false; }
  }

  async function run(key: string, fn: () => Promise<unknown>, done: string) {
    busy = { ...busy, [key]: true }; error = ''; notice = '';
    try { await fn(); notice = done; await load(); }
    catch (e) { error = errorMessage(e); }
    finally { busy = { ...busy, [key]: false }; }
  }

  const save = (short: string, expansion: string) =>
    run(short, () => abbreviations.save(short, expansion), `“${short}” saved as “${expansion.trim()}”.`)
      .then(() => { editing = null; draft = { ...draft, [short]: '' }; });
  const ignore = (short: string) => run(short, () => abbreviations.ignore(short), `“${short}” marked as not an abbreviation.`);
  const remove = (e: AbbreviationEntry) =>
    run(e.short, () => abbreviations.remove(e.id), e.status === 'ignored' ? `“${e.short}” is a candidate again.` : `“${e.short}” removed.`);

  async function addNew() {
    const s = newShort.trim(), x = newExpansion.trim();
    if (!s || !x) return;
    await save(s, x);
    newShort = ''; newExpansion = '';
  }

  $: active = entries.filter(e => e.status === 'active');
  $: ignored = entries.filter(e => e.status === 'ignored');
  $: unansweredFirst = candidates.filter(c => c.unanswered > 0).length;
</script>

<svelte:head><title>Abbreviations · Admin</title></svelte:head>

<PageHeader title="Abbreviations" description="Short forms students type (sempro, TA, tendik). Explain them here so the search understands them.">
  <button on:click={load} class="btn-outline" disabled={loading}>
    <RefreshCw size={15} class={loading ? 'animate-spin' : ''} /> <span class="hidden sm:inline">Refresh</span>
  </button>
</PageHeader>

<div class="space-y-6">
  {#if notice}<Alert variant="success" dismissible on:dismiss={() => (notice = '')}>{notice}</Alert>{/if}
  {#if error}<Alert dismissible on:dismiss={() => (error = '')}>{error}</Alert>{/if}

  <div class="rounded-lg border px-4 py-3 text-sm {expandEnabled ? 'bg-primary/5' : 'bg-muted/40'}">
    {#if expandEnabled}
      <strong>Expansion is on.</strong> Questions are searched with these abbreviations explained.
    {:else}
      <strong>Expansion is off.</strong> The list below is kept but not used yet. Turn it on in
      <a href="/admin/settings" class="text-primary hover:underline">Settings</a> after measuring it with
      <code class="text-xs">eval_retrieval.py --expand</code>.
    {/if}
  </div>

  <!-- Candidates -->
  <section class="card overflow-hidden">
    <div class="flex flex-wrap items-center gap-3 px-5 py-4 border-b">
      <div class="flex-1 min-w-0">
        <h2 class="font-bold">Found in questions</h2>
        <p class="text-xs text-muted-foreground">Words in capitals, or short words the documents do not contain. Most often unanswered first.</p>
      </div>
      <select bind:value={days} on:change={load} class="input w-40" aria-label="Period">
        <option value={30}>Last 30 days</option>
        <option value={90}>Last 90 days</option>
        <option value={180}>Last 180 days</option>
      </select>
    </div>
    {#if loading && !candidates.length}
      <LoadingState />
    {:else if !candidates.length}
      <EmptyState icon={BookA} title="No new abbreviations" description="Every short form in the questions of this period is explained or marked as not an abbreviation." />
    {:else}
      {#if unansweredFirst}<p class="px-5 pt-3 text-xs text-muted-foreground">{unansweredFirst} of {candidates.length} appear in unanswered questions.</p>{/if}
      <ul class="divide-y">
        {#each candidates as c (c.short)}
          <li class="px-5 py-3.5 flex flex-col lg:flex-row lg:items-center gap-3">
            <div class="lg:w-72 min-w-0">
              <p class="flex flex-wrap items-center gap-1.5">
                <span class="font-mono font-semibold">{c.short}</span>
                <span class="badge-neutral">{c.times}×</span>
                {#if c.unanswered}<span class="badge-warning">{c.unanswered}× unanswered</span>{/if}
                {#if c.in_documents}<span class="badge-neutral" title="The word also appears in a document">in documents</span>{/if}
              </p>
              {#each c.examples.slice(0, 2) as ex}
                <p class="text-xs text-muted-foreground truncate mt-0.5" title={ex}>“{ex}”</p>
              {/each}
            </div>
            <form class="flex-1 flex gap-2" on:submit|preventDefault={() => save(c.short, draft[c.short] ?? '')}>
              <input bind:value={draft[c.short]} placeholder="Meaning, e.g. seminar proposal" class="input flex-1" aria-label="Meaning of {c.short}" maxlength="120" />
              <button type="submit" class="btn-primary btn-sm shrink-0" disabled={busy[c.short] || !(draft[c.short] ?? '').trim()}>
                <Check size={13} /> Save
              </button>
              <button type="button" class="btn-ghost btn-sm shrink-0" on:click={() => ignore(c.short)} disabled={busy[c.short]} title="Not an abbreviation: hide it">
                <EyeOff size={13} /> <span class="hidden sm:inline">Not one</span>
              </button>
            </form>
          </li>
        {/each}
      </ul>
    {/if}
  </section>

  <!-- Dictionary -->
  <section class="card overflow-hidden">
    <div class="px-5 py-4 border-b">
      <h2 class="font-bold">Explained abbreviations <span class="text-sm font-normal text-muted-foreground">· {active.length}</span></h2>
      <p class="text-xs text-muted-foreground">Used when searching (if expansion is on). Not case-sensitive.</p>
    </div>
    <form class="flex flex-col sm:flex-row gap-2 px-5 py-3 border-b bg-muted/20" on:submit|preventDefault={addNew}>
      <input bind:value={newShort} placeholder="Short form, e.g. yudis" class="input sm:w-48" aria-label="New short form" maxlength="16" />
      <input bind:value={newExpansion} placeholder="Meaning, e.g. yudisium" class="input flex-1" aria-label="Its meaning" maxlength="120" />
      <button type="submit" class="btn-outline btn-sm shrink-0" disabled={!newShort.trim() || !newExpansion.trim()}><Plus size={13} /> Add</button>
    </form>
    {#if !active.length}
      <p class="px-5 py-6 text-sm text-muted-foreground text-center">None yet. Explain a candidate above or add one.</p>
    {:else}
      <ul class="divide-y">
        {#each active as e (e.id)}
          <li class="px-5 py-3 flex items-center gap-3">
            <span class="font-mono font-semibold w-28 shrink-0 truncate">{e.short}</span>
            {#if editing === e.short}
              <form class="flex-1 flex gap-2" on:submit|preventDefault={() => save(e.short, draft[e.short] ?? e.expansion)}>
                <input bind:value={draft[e.short]} class="input flex-1" aria-label="Meaning of {e.short}" maxlength="120" />
                <button type="submit" class="btn-primary btn-sm" disabled={busy[e.short]}><Check size={13} /> Save</button>
                <button type="button" class="btn-ghost btn-sm" on:click={() => (editing = null)}>Cancel</button>
              </form>
            {:else}
              <span class="flex-1 min-w-0 text-sm">{e.expansion}</span>
              <span class="hidden sm:inline text-xs text-muted-foreground shrink-0">{formatDate(e.updated_at)}</span>
              <button class="icon-btn" title="Edit" aria-label="Edit {e.short}" on:click={() => { editing = e.short; draft = { ...draft, [e.short]: e.expansion }; }}><Pencil size={14} /></button>
              <button class="icon-btn hover:!text-destructive" title="Remove" aria-label="Remove {e.short}" on:click={() => remove(e)} disabled={busy[e.short]}><Trash2 size={14} /></button>
            {/if}
          </li>
        {/each}
      </ul>
    {/if}
    {#if ignored.length}
      <div class="px-5 py-3 border-t text-xs text-muted-foreground">
        Not abbreviations ({ignored.length}):
        {#each ignored as e, i (e.id)}
          <button class="font-mono hover:text-foreground hover:underline" title="Make it a candidate again" on:click={() => remove(e)}>{e.short}</button>{i < ignored.length - 1 ? ', ' : ''}
        {/each}
      </div>
    {/if}
  </section>

  <!-- Built-in -->
  <section class="card overflow-hidden">
    <button type="button" class="w-full flex items-center gap-2 px-5 py-4 text-left" on:click={() => (showBuiltin = !showBuiltin)} aria-expanded={showBuiltin}>
      <ChevronDown size={16} class="text-muted-foreground transition-transform {showBuiltin ? '' : '-rotate-90'}" />
      <span class="font-bold">Built-in list</span>
      <span class="text-sm text-muted-foreground">· {builtin.length} · also used when documents are ingested (src/acronyms.json)</span>
    </button>
    {#if showBuiltin}
      <ul class="border-t divide-y max-h-96 overflow-y-auto">
        {#each builtin as b (b.short)}
          <li class="px-5 py-2 flex gap-3 text-sm"><span class="font-mono w-28 shrink-0">{b.short}</span><span class="text-muted-foreground">{b.expansion}</span></li>
        {/each}
      </ul>
    {/if}
  </section>
</div>

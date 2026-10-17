<script lang="ts">
  import { onMount } from 'svelte';
  import { faq } from '$lib/api/client';
  import type { FaqPin, TopQuery } from '$lib/api/client';
  import { errorMessage, formatDate } from '$lib/format';
  import PageHeader from '$lib/components/ui/PageHeader.svelte';
  import Alert from '$lib/components/ui/Alert.svelte';
  import LoadingState from '$lib/components/ui/LoadingState.svelte';
  import { Loader2, Pin, PinOff, TrendingUp, RefreshCw, Trash2, Plus } from 'lucide-svelte';

  let pins: FaqPin[] = [];
  let topQ: TopQuery[] = [];
  let loading = false;
  let saving = false;
  let error = '';
  let success = '';
  let newQuestion = '';

  onMount(load);

  async function load() {
    loading = true; error = '';
    try { [pins, topQ] = await Promise.all([faq.pins(), faq.topQueries()]); }
    catch (e) { error = errorMessage(e); }
    finally { loading = false; }
  }

  async function run(fn: () => Promise<unknown>, message: string) {
    saving = true; error = ''; success = '';
    try {
      await fn();
      success = message;
      await load();
    } catch (e) { error = errorMessage(e); }
    finally { saving = false; }
  }

  const pinQuestion = (q: string) => run(() => faq.pin(q), 'Question pinned.');
  const unpinQuestion = (id: number) => run(() => faq.unpin(id), 'Question unpinned.');

  async function addManual() {
    const q = newQuestion.trim();
    if (!q) return;
    await run(() => faq.pin(q), 'Question added.');
    if (!error) newQuestion = '';
  }

  $: pinnedByText = new Map(pins.map(p => [p.question, p]));
</script>

<svelte:head><title>FAQ · Admin</title></svelte:head>

<PageHeader title="FAQ" description="Choose the questions shown on the home page. Up to 8 are shown — pinned first, then the most asked.">
  <button on:click={load} disabled={loading} class="btn-outline">
    <RefreshCw size={15} class={loading ? 'animate-spin' : ''} /> Refresh
  </button>
</PageHeader>

<div class="space-y-4 mb-5">
  {#if error}<Alert dismissible on:dismiss={() => (error = '')}>{error}</Alert>{/if}
  {#if success}<Alert variant="success" dismissible on:dismiss={() => (success = '')}>{success}</Alert>{/if}
</div>

{#if loading && !pins.length && !topQ.length}
  <LoadingState />
{:else}
  <div class="grid lg:grid-cols-2 gap-5 items-start">
    <!-- Pinned -->
    <section class="card overflow-hidden">
      <header class="flex items-center justify-between gap-2 px-5 py-4 border-b">
        <h2 class="text-sm font-bold flex items-center gap-2"><Pin size={15} class="text-primary" /> Pinned</h2>
        <span class="badge-primary">{pins.length}</span>
      </header>

      <form on:submit|preventDefault={addManual} class="flex gap-2 p-4 border-b bg-muted/30">
        <input bind:value={newQuestion} placeholder="Add a question manually…" class="input" aria-label="New FAQ question" />
        <button type="submit" disabled={saving || !newQuestion.trim()} class="btn-primary shrink-0 px-3" aria-label="Add question">
          {#if saving}<Loader2 size={15} class="animate-spin" />{:else}<Plus size={16} />{/if}
          <span class="hidden sm:inline">Add</span>
        </button>
      </form>

      {#if pins.length === 0}
        <p class="px-5 py-10 text-sm text-muted-foreground text-center">
          Nothing pinned yet. Pin a question from the most asked list, or add one above.
        </p>
      {:else}
        <ul class="divide-y">
          {#each pins as p (p.id)}
            <li class="flex items-start gap-3 px-5 py-3.5">
              <div class="flex-1 min-w-0">
                <p class="text-sm leading-snug">{p.question}</p>
                <p class="text-[11px] text-muted-foreground mt-1">Pinned {formatDate(p.created_at)}</p>
              </div>
              <button on:click={() => unpinQuestion(p.id)} disabled={saving}
                class="icon-btn hover:!text-destructive hover:!bg-destructive/10" title="Unpin" aria-label="Unpin question">
                <Trash2 size={15} />
              </button>
            </li>
          {/each}
        </ul>
      {/if}
    </section>

    <!-- Most asked -->
    <section class="card overflow-hidden">
      <header class="flex items-center justify-between gap-2 px-5 py-4 border-b">
        <h2 class="text-sm font-bold flex items-center gap-2"><TrendingUp size={15} class="text-warning" /> Most asked</h2>
        <span class="text-xs text-muted-foreground">Pin to feature on the home page</span>
      </header>
      {#if topQ.length === 0}
        <p class="px-5 py-10 text-sm text-muted-foreground text-center">
          No queries yet. This list fills up once students start asking.
        </p>
      {:else}
        <ul class="divide-y">
          {#each topQ as q}
            {@const pin = pinnedByText.get(q.query_text)}
            <li class="flex items-start gap-3 px-5 py-3.5">
              <span class="badge-warning shrink-0 mt-0.5 min-w-[2.75rem] justify-center">{q.count}×</span>
              <div class="flex-1 min-w-0">
                <p class="text-sm leading-snug {pin ? 'text-muted-foreground' : ''}">{q.query_text}</p>
                <p class="text-[11px] text-muted-foreground mt-1">Last asked {formatDate(q.last_asked)}</p>
              </div>
              <button
                on:click={() => (pin ? unpinQuestion(pin.id) : pinQuestion(q.query_text))}
                disabled={saving}
                class="btn-sm shrink-0 {pin ? 'btn-outline text-destructive' : 'btn-outline text-primary'}">
                {#if pin}<PinOff size={13} /> Unpin{:else}<Pin size={13} /> Pin{/if}
              </button>
            </li>
          {/each}
        </ul>
      {/if}
    </section>
  </div>
{/if}

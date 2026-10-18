<script lang="ts">
  import { onMount } from 'svelte';
  import { slide } from 'svelte/transition';
  import { admin } from '$lib/api/client';
  import type { SourceRow, DraftDocument, DraftChunk, IngestJob, NewSourcePayload } from '$lib/api/client';
  import { errorMessage, formatDateTime, hostname } from '$lib/format';
  import PageHeader from '$lib/components/ui/PageHeader.svelte';
  import Alert from '$lib/components/ui/Alert.svelte';
  import Modal from '$lib/components/ui/Modal.svelte';
  import LoadingState from '$lib/components/ui/LoadingState.svelte';
  import EmptyState from '$lib/components/ui/EmptyState.svelte';
  import {
    Loader2, RefreshCw, Plus, Trash2, ExternalLink, FolderOpen, FileText, Globe, Check, X,
    ChevronDown, Clock, AlertTriangle, CircleCheck, Image, Table2, FolderSync
  } from 'lucide-svelte';

  let sources: SourceRow[] = [];
  let drafts: DraftDocument[] = [];
  let loading = false;
  let error = '';
  let notice = '';

  // Add source
  let showAdd = false;
  let form: NewSourcePayload = blankForm();
  let adding = false;
  let addError = '';

  // Draft review
  let openDraft: string | null = null;
  let draftChunks: Record<string, DraftChunk[]> = {};
  let draftBusy: Record<string, boolean> = {};

  // Jobs per source
  let openJobs: number | null = null;
  let jobs: Record<number, IngestJob[]> = {};

  const kindInfo = {
    drive_folder: { label: 'Drive folder', icon: FolderOpen },
    drive_file:   { label: 'Drive file',   icon: FileText },
    web_pdf:      { label: 'Website PDF',  icon: Globe }
  } as const;

  function blankForm(): NewSourcePayload {
    return { url: '', title: '', category: '', subcategory: '', version_label: '', valid_from: '', supersedes_id: null, auto_check: true };
  }

  onMount(load);

  async function load() {
    loading = true; error = '';
    try {
      [sources, drafts] = await Promise.all([admin.sources(), admin.drafts()]);
    } catch (e) {
      error = errorMessage(e);
    } finally {
      loading = false;
    }
  }

  function detectKind(url: string): string {
    if (/drive\.google\.com\/drive\/(u\/\d+\/)?folders\//.test(url)) return 'Google Drive folder — every PDF inside is ingested';
    if (/(drive|docs)\.google\.com/.test(url)) return 'Google Drive file';
    if (/^https?:\/\//.test(url)) return 'PDF on a website (the link must open the .pdf file itself)';
    return '';
  }
  $: kindHint = detectKind(form.url.trim());

  async function addSource() {
    adding = true; addError = '';
    try {
      const payload: NewSourcePayload = { ...form, url: form.url.trim() };
      if (!payload.valid_from) delete payload.valid_from;
      if (!payload.supersedes_id) payload.supersedes_id = null;
      const res = await admin.createSource(payload);
      showAdd = false;
      notice = res.message;
      form = blankForm();
      await load();
    } catch (e) {
      addError = errorMessage(e);
    } finally {
      adding = false;
    }
  }

  async function check(src: SourceRow) {
    try {
      const res = await admin.checkSource(src.id);
      notice = res.message ?? (res.queued ? 'Queued.' : 'Already waiting in the queue.');
      await load();
    } catch (e) { error = errorMessage(e); }
  }

  async function remove(src: SourceRow) {
    if (!confirm(`Remove "${src.title || src.url}"?\n\nDrafts are deleted. Approved documents from this source are archived (no longer used in answers).`)) return;
    try {
      await admin.deleteSource(src.id);
      notice = 'Source removed.';
      await load();
    } catch (e) { error = errorMessage(e); }
  }

  async function toggleJobs(src: SourceRow) {
    if (openJobs === src.id) { openJobs = null; return; }
    openJobs = src.id;
    try { jobs = { ...jobs, [src.id]: await admin.sourceJobs(src.id) }; }
    catch (e) { error = errorMessage(e); }
  }

  async function toggleDraft(d: DraftDocument) {
    if (openDraft === d.document_id) { openDraft = null; return; }
    openDraft = d.document_id;
    if (!draftChunks[d.document_id]) {
      try { draftChunks = { ...draftChunks, [d.document_id]: await admin.draftChunks(d.document_id) }; }
      catch (e) { error = errorMessage(e); }
    }
  }

  async function approve(d: DraftDocument) {
    const replaces = d.replaces.length
      ? `\n\nThese documents will be archived (replaced):\n• ${d.replaces.join('\n• ')}`
      : '';
    if (!confirm(`Approve "${d.title}"? It will be used in answers right away.${replaces}`)) return;
    draftBusy = { ...draftBusy, [d.document_id]: true };
    try {
      const res = await admin.approveDraft(d.document_id);
      notice = `Approved: ${res.activated_chunks} chunks are now searchable` +
        (res.archived_documents.length ? `; ${res.archived_documents.length} older version(s) archived.` : '.');
      await load();
    } catch (e) { error = errorMessage(e); }
    finally { draftBusy = { ...draftBusy, [d.document_id]: false }; }
  }

  async function reject(d: DraftDocument) {
    if (!confirm(`Reject "${d.title}"? The draft is deleted. The file is processed again only when it changes or you press "Check now".`)) return;
    draftBusy = { ...draftBusy, [d.document_id]: true };
    try {
      await admin.rejectDraft(d.document_id);
      notice = 'Draft rejected.';
      await load();
    } catch (e) { error = errorMessage(e); }
    finally { draftBusy = { ...draftBusy, [d.document_id]: false }; }
  }

  function warnings(log: string | null): string[] {
    return (log ?? '').split('\n').filter(l => /angka di PDF|tanpa teks|gagal|GAGAL|duplikat|sama dengan|tidak dideskripsikan|hanya \d+ pertama/.test(l));
  }

  const stateStyle: Record<string, string> = {
    queued: 'badge-neutral', running: 'badge-primary', done: 'badge-success', failed: 'badge-danger'
  };
</script>

<svelte:head><title>Sources · Admin</title></svelte:head>

<!-- Add source -->
<Modal open={showAdd} title="Add a source" on:close={() => (showAdd = false)}>
  <form id="add-source" on:submit|preventDefault={addSource} class="space-y-4">
    <div>
      <label for="src-url" class="label">Link</label>
      <input id="src-url" bind:value={form.url} required class="input" inputmode="url"
        placeholder="https://drive.google.com/drive/folders/… or https://…/panduan.pdf" />
      {#if kindHint}<p class="text-xs text-muted-foreground mt-1.5">{kindHint}</p>{/if}
      <p class="text-xs text-muted-foreground mt-1.5">
        Drive files and folders must be shared as <strong>Anyone with the link can view</strong>.
        If the same PDF is also on the department website, add the website link too — answers will cite the official link.
      </p>
    </div>
    <div>
      <label for="src-title" class="label">Title</label>
      <input id="src-title" bind:value={form.title} class="input" placeholder="e.g. Buku Panduan Akademik MTI" />
    </div>
    <div class="grid sm:grid-cols-2 gap-3">
      <div>
        <label for="src-version" class="label">Version</label>
        <input id="src-version" bind:value={form.version_label} class="input" placeholder="e.g. 2022 Revisi-2" />
      </div>
      <div>
        <label for="src-valid" class="label">Valid from</label>
        <input id="src-valid" type="date" bind:value={form.valid_from} class="input" />
      </div>
      <div>
        <label for="src-cat" class="label">Category</label>
        <input id="src-cat" bind:value={form.category} class="input" placeholder="e.g. akademik" />
      </div>
      <div>
        <label for="src-subcat" class="label">Subcategory</label>
        <input id="src-subcat" bind:value={form.subcategory} class="input" placeholder="e.g. panduan" />
      </div>
    </div>
    <div>
      <label for="src-supersedes" class="label">Replaces an older source (optional)</label>
      <select id="src-supersedes" bind:value={form.supersedes_id} class="input">
        <option value={null}>— none —</option>
        {#each sources as s}<option value={s.id}>{s.title || s.url}{s.version_label ? ` (${s.version_label})` : ''}</option>{/each}
      </select>
      <p class="text-xs text-muted-foreground mt-1.5">When you approve the new version, the old source's documents are archived.</p>
    </div>
    <label class="flex items-center gap-2 text-sm">
      <input type="checkbox" bind:checked={form.auto_check} class="rounded border" />
      Check for changes automatically (daily)
    </label>
    {#if addError}<Alert>{addError}</Alert>{/if}
  </form>
  <svelte:fragment slot="footer">
    <button type="button" class="btn-outline" disabled={adding} on:click={() => (showAdd = false)}>Cancel</button>
    <button type="submit" form="add-source" class="btn-primary" disabled={adding || !form.url.trim()}>
      {#if adding}<Loader2 size={15} class="animate-spin" />{:else}<Plus size={15} />{/if} Add source
    </button>
  </svelte:fragment>
</Modal>

<PageHeader title="Sources" description="PDF documents from Google Drive and the department website. New and changed files wait here for review before they are used in answers.">
  <button on:click={load} class="btn-outline" disabled={loading}>
    <RefreshCw size={15} class={loading ? 'animate-spin' : ''} /> <span class="hidden sm:inline">Refresh</span>
  </button>
  <button on:click={() => { addError = ''; showAdd = true; }} class="btn-primary"><Plus size={15} /> Add source</button>
</PageHeader>

<div class="space-y-8">
  {#if notice}<Alert variant="success" dismissible on:dismiss={() => (notice = '')}>{notice}</Alert>{/if}
  {#if error}<Alert dismissible on:dismiss={() => (error = '')}>{error}</Alert>{/if}

  <!-- Review queue -->
  <section aria-labelledby="review-heading" class="space-y-3">
    <h2 id="review-heading" class="text-sm font-bold uppercase tracking-[0.12em] text-muted-foreground">
      Waiting for review {#if drafts.length}<span class="badge-primary ml-1">{drafts.length}</span>{/if}
    </h2>
    {#if loading && !drafts.length && !sources.length}
      <LoadingState />
    {:else if !drafts.length}
      <div class="card p-4 text-sm text-muted-foreground flex items-center gap-2">
        <CircleCheck size={16} class="text-success" /> Nothing to review.
      </div>
    {:else}
      <ul class="card divide-y overflow-hidden">
        {#each drafts as d (d.document_id)}
          {@const open = openDraft === d.document_id}
          {@const warn = warnings(d.job_log)}
          <li class="p-4 space-y-3">
            <div class="flex flex-col sm:flex-row sm:items-start gap-3">
              <button on:click={() => toggleDraft(d)} aria-expanded={open} class="flex-1 min-w-0 text-left flex gap-3">
                <span class="w-9 h-9 rounded-lg bg-primary/10 text-primary flex items-center justify-center shrink-0"><FileText size={17} /></span>
                <span class="min-w-0 flex-1">
                  <span class="block font-semibold text-sm leading-snug">{d.source_title || d.title}</span>
                  <span class="flex flex-wrap items-center gap-1.5 mt-1.5 text-xs">
                    {#if d.version_label}<span class="badge-primary">{d.version_label}</span>{/if}
                    <span class="badge-neutral">{d.page_count} pages</span>
                    <span class="badge-neutral">{d.chunk_count} chunks</span>
                    {#if d.table_chunks}<span class="badge-neutral"><Table2 size={10} /> {d.table_chunks}</span>{/if}
                    {#if d.figure_chunks}<span class="badge-neutral"><Image size={10} /> {d.figure_chunks}</span>{/if}
                    {#if warn.length}<span class="badge-warning"><AlertTriangle size={10} /> {warn.length} warnings</span>{/if}
                  </span>
                  <span class="block text-xs text-muted-foreground mt-1 font-mono truncate">{d.document_id}</span>
                </span>
                <ChevronDown size={16} class="shrink-0 mt-2 text-muted-foreground transition-transform {open ? 'rotate-180' : ''}" />
              </button>
              <div class="flex items-center gap-2 shrink-0">
                {#if d.citation_url}
                  <a href={d.citation_url} target="_blank" rel="noopener noreferrer" class="icon-btn" title="Open the original" aria-label="Open the original"><ExternalLink size={15} /></a>
                {/if}
                <button class="btn-outline btn-sm" disabled={draftBusy[d.document_id]} on:click={() => reject(d)}><X size={14} /> Reject</button>
                <button class="btn-primary btn-sm" disabled={draftBusy[d.document_id]} on:click={() => approve(d)}>
                  {#if draftBusy[d.document_id]}<Loader2 size={14} class="animate-spin" />{:else}<Check size={14} />{/if} Approve
                </button>
              </div>
            </div>

            {#if d.replaces.length}
              <Alert variant="info">Approving archives the current version: {d.replaces.join(', ')}</Alert>
            {/if}

            {#if open}
              <div class="space-y-3" transition:slide={{ duration: 150 }}>
                {#if warn.length}
                  <Alert variant="warning">
                    <p class="font-semibold mb-1">Check these against the original before approving:</p>
                    <ul class="list-disc pl-4 space-y-0.5">{#each warn as w}<li>{w}</li>{/each}</ul>
                  </Alert>
                {/if}
                {#if !draftChunks[d.document_id]}
                  <p class="flex items-center gap-2 text-sm text-muted-foreground"><Loader2 size={14} class="animate-spin" /> Loading chunks…</p>
                {:else}
                  <div class="max-h-[28rem] overflow-y-auto space-y-1.5 pr-1">
                    {#each draftChunks[d.document_id] as c (c.id)}
                      <details class="rounded-md border bg-muted/30 text-xs" open={c.chunk_kind === 'figure'}>
                        <summary class="cursor-pointer px-2.5 py-1.5 flex gap-2 min-w-0">
                          <span class="font-mono text-muted-foreground shrink-0">p.{c.page_number ?? '–'}</span>
                          {#if c.chunk_kind !== 'text'}<span class="badge-primary shrink-0">{c.chunk_kind}</span>{/if}
                          <span class="truncate flex-1">{c.content.split('\n')[0]}</span>
                        </summary>
                        <pre class="whitespace-pre-wrap break-words font-sans px-2.5 pb-2 leading-relaxed">{c.content}</pre>
                      </details>
                    {/each}
                  </div>
                {/if}
              </div>
            {/if}
          </li>
        {/each}
      </ul>
    {/if}
  </section>

  <!-- Sources -->
  <section aria-labelledby="sources-heading" class="space-y-3">
    <h2 id="sources-heading" class="text-sm font-bold uppercase tracking-[0.12em] text-muted-foreground">Sources</h2>
    {#if !sources.length && !loading}
      <div class="card">
        <EmptyState icon={FolderSync} title="No sources yet"
          description="Add a shared Google Drive folder or a PDF link from the department website." />
      </div>
    {:else}
      <ul class="card divide-y overflow-hidden">
        {#each sources as s (s.id)}
          {@const info = kindInfo[s.kind] ?? kindInfo.web_pdf}
          <li class="p-4">
            <div class="flex flex-col sm:flex-row sm:items-start gap-3">
              <div class="flex-1 min-w-0 flex gap-3">
                <span class="w-9 h-9 rounded-lg bg-muted text-muted-foreground flex items-center justify-center shrink-0">
                  <svelte:component this={info.icon} size={17} />
                </span>
                <div class="min-w-0">
                  <p class="font-semibold text-sm leading-snug">{s.title || hostname(s.url) || s.url}</p>
                  <p class="flex flex-wrap items-center gap-1.5 mt-1.5 text-xs">
                    <span class="badge-neutral">{info.label}</span>
                    {#if s.version_label}<span class="badge-primary">{s.version_label}</span>{/if}
                    {#if s.category}<span class="badge-neutral">{s.category}{s.subcategory ? ` / ${s.subcategory}` : ''}</span>{/if}
                    <span class="badge-neutral">{s.file_count} {s.file_count === 1 ? 'file' : 'files'} · {s.active_files} active · {s.draft_files} in review</span>
                    {#if s.last_job_state}<span class={stateStyle[s.last_job_state] ?? 'badge-neutral'}>{s.last_job_state}</span>{/if}
                  </p>
                  <p class="text-xs text-muted-foreground mt-1 flex items-center gap-1">
                    <Clock size={11} /> {s.last_checked ? `checked ${formatDateTime(s.last_checked)}` : 'not checked yet'}
                    {#if s.auto_check}· auto daily{/if}
                  </p>
                  {#if s.last_job_state === 'failed' && s.last_job_log}
                    <div class="mt-2"><Alert>{s.last_job_log.split('\n').slice(-1)[0]}</Alert></div>
                  {/if}
                </div>
              </div>
              <div class="flex items-center gap-1 shrink-0">
                <a href={s.url} target="_blank" rel="noopener noreferrer" class="icon-btn" title="Open link" aria-label="Open link"><ExternalLink size={15} /></a>
                <button class="btn-outline btn-sm" on:click={() => check(s)} disabled={s.last_job_state === 'queued' || s.last_job_state === 'running'}>
                  <RefreshCw size={13} /> Check now
                </button>
                <button class="btn-ghost btn-sm" on:click={() => toggleJobs(s)} aria-expanded={openJobs === s.id}>History</button>
                <button class="icon-btn hover:!text-destructive hover:!bg-destructive/10" on:click={() => remove(s)} title="Remove source" aria-label="Remove source"><Trash2 size={15} /></button>
              </div>
            </div>
            {#if openJobs === s.id}
              <div class="mt-3 sm:pl-12 space-y-2" transition:slide={{ duration: 150 }}>
                {#if !jobs[s.id]}
                  <p class="flex items-center gap-2 text-sm text-muted-foreground"><Loader2 size={14} class="animate-spin" /> Loading…</p>
                {:else if !jobs[s.id].length}
                  <p class="text-sm text-muted-foreground">No runs yet.</p>
                {:else}
                  {#each jobs[s.id] as j (j.id)}
                    <div class="rounded-lg border bg-muted/30 p-3">
                      <p class="text-xs flex flex-wrap gap-2 items-center">
                        <span class={stateStyle[j.state] ?? 'badge-neutral'}>{j.state}</span>
                        <span class="text-muted-foreground">{formatDateTime(j.finished_at ?? j.created_at)}</span>
                      </p>
                      {#if j.log}<pre class="mt-2 text-xs whitespace-pre-wrap break-words font-sans leading-relaxed">{j.log}</pre>{/if}
                    </div>
                  {/each}
                {/if}
              </div>
            {/if}
          </li>
        {/each}
      </ul>
    {/if}
  </section>
</div>

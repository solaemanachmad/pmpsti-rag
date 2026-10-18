<script lang="ts">
  import { onMount } from 'svelte';
  import { slide } from 'svelte/transition';
  import { admin } from '$lib/api/client';
  import type { AdminDocument, AdminChunk, IngestUrlPayload, UrlPreview } from '$lib/api/client';
  import { errorMessage, hostname } from '$lib/format';
  import PageHeader from '$lib/components/ui/PageHeader.svelte';
  import Alert from '$lib/components/ui/Alert.svelte';
  import Modal from '$lib/components/ui/Modal.svelte';
  import LoadingState from '$lib/components/ui/LoadingState.svelte';
  import EmptyState from '$lib/components/ui/EmptyState.svelte';
  import { Loader2, Trash2, RefreshCw, Globe, ChevronDown, Pencil, Search, FileText, ExternalLink, Layers, Eye } from 'lucide-svelte';

  let documents: AdminDocument[] = [];
  let loading = false;
  let error = '';
  let notice = '';
  let search = '';
  let category = '';

  // Ingest
  let showIngest = false;
  let ingestUrl = ''; let ingestTitle = ''; let ingestCategory = ''; let ingestSubcategory = '';
  let ingestChunkStrategy = 'auto';
  let ingestLoading = false; let ingestError = '';
  // Preview: what would be saved, plus quality warnings. Cleared when URL, title or
  // strategy change, because those change the chunks.
  let preview: UrlPreview | null = null;
  let previewLoading = false;
  let previewKey = '';

  const chunkStrategyOptions = [
    { value: 'auto',       label: 'Auto',       desc: 'Detects the best strategy from the content (recommended).' },
    { value: 'sentence',   label: 'Sentence',   desc: 'Best for articles and prose. Uses no Gemini quota while chunking.' },
    { value: 'structural', label: 'Structural', desc: 'Best for numbered lists, tables and staff directories.' },
    { value: 'semantic',   label: 'Semantic',   desc: 'Most accurate, but uses more Gemini quota.' }
  ];

  // Chunks
  let expandedDoc: string | null = null;
  let chunksByDoc: Record<string, AdminChunk[]> = {};
  let chunksLoading: Record<string, boolean> = {};
  let chunksError: Record<string, string> = {};

  // Edit
  let editing: AdminDocument | null = null;
  let editTitle = ''; let editCategory = ''; let editSubcategory = '';
  let savingDoc = false; let editError = '';

  onMount(loadDocs);

  async function loadDocs() {
    loading = true; error = '';
    try { documents = await admin.documents(); }
    catch (e) { error = errorMessage(e); }
    finally { loading = false; }
  }

  function refresh() {
    chunksByDoc = {};
    loadDocs();
  }

  async function toggleChunks(docId: string) {
    if (expandedDoc === docId) { expandedDoc = null; return; }
    expandedDoc = docId;
    if (chunksByDoc[docId]) return;
    chunksLoading = { ...chunksLoading, [docId]: true };
    chunksError = { ...chunksError, [docId]: '' };
    try {
      chunksByDoc = { ...chunksByDoc, [docId]: await admin.listChunks(docId) };
    } catch (e) {
      chunksError = { ...chunksError, [docId]: errorMessage(e) };
    } finally {
      chunksLoading = { ...chunksLoading, [docId]: false };
    }
  }

  async function deleteDoc(doc: AdminDocument) {
    if (!confirm(`Delete "${doc.title || doc.document_id}"? All of its chunks will be removed.`)) return;
    try {
      await admin.deleteDocument(doc.document_id);
      if (expandedDoc === doc.document_id) expandedDoc = null;
      await loadDocs();
    } catch (e) { error = errorMessage(e); }
  }

  function startEdit(doc: AdminDocument) {
    editing = doc;
    editTitle = doc.title;
    editCategory = doc.category;
    editSubcategory = doc.subcategory ?? '';
    editError = '';
  }

  async function saveDoc() {
    if (!editing) return;
    const id = editing.document_id;
    const patch = { title: editTitle.trim(), category: editCategory.trim(), subcategory: editSubcategory.trim() };
    savingDoc = true; editError = '';
    try {
      await admin.updateDocument(id, patch);
      documents = documents.map(d => (d.document_id === id ? { ...d, ...patch } : d));
      const { [id]: _, ...rest } = chunksByDoc;
      chunksByDoc = rest;
      editing = null;
    } catch (e) { editError = errorMessage(e); }
    finally { savingDoc = false; }
  }

  function openIngest() {
    ingestError = '';
    showIngest = true;
  }

  function ingestPayload(): IngestUrlPayload {
    return {
      url: ingestUrl.trim(),
      title: ingestTitle.trim() || undefined,
      category: ingestCategory.trim() || undefined,
      subcategory: ingestSubcategory.trim() || undefined,
      chunk_strategy: ingestChunkStrategy
    };
  }

  $: currentKey = JSON.stringify([ingestUrl.trim(), ingestTitle.trim(), ingestChunkStrategy]);
  $: if (preview && currentKey !== previewKey) preview = null;

  async function doPreview() {
    if (!ingestUrl.trim()) { ingestError = 'URL is required.'; return; }
    previewLoading = true; ingestError = ''; preview = null;
    try {
      const key = currentKey;
      preview = await admin.previewUrl(ingestPayload());
      previewKey = key;
    } catch (e) { ingestError = errorMessage(e); }
    finally { previewLoading = false; }
  }

  async function doIngest() {
    if (!ingestUrl.trim()) { ingestError = 'URL is required.'; return; }
    ingestLoading = true; ingestError = ''; notice = '';
    try {
      const res = await admin.ingestUrl(ingestPayload());
      notice = `Indexed “${res.title}” — ${res.chunks} chunks.`;
      ingestUrl = ingestTitle = ingestCategory = ingestSubcategory = '';
      ingestChunkStrategy = 'auto';
      preview = null;
      showIngest = false;
      await loadDocs();
    } catch (e) { ingestError = errorMessage(e); }
    finally { ingestLoading = false; }
  }

  $: categories = [...new Set(documents.map(d => d.category).filter(Boolean))].sort();
  $: totalChunks = documents.reduce((n, d) => n + (d.chunk_count ?? 0), 0);
  $: q = search.trim().toLowerCase();
  $: filtered = documents.filter(d =>
    (!category || d.category === category) &&
    (!q || [d.title, d.source_url, d.category, d.subcategory].some(v => v?.toLowerCase().includes(q)))
  );
</script>

<svelte:head><title>Documents · Admin</title></svelte:head>

<!-- Ingest modal -->
<Modal open={showIngest} title="Add a source from URL" size="lg" on:close={() => !ingestLoading && !previewLoading && (showIngest = false)}>
  <form id="ingest" on:submit|preventDefault={doIngest} class="space-y-4">
    <div>
      <label for="ingest-url" class="label">URL</label>
      <input id="ingest-url" bind:value={ingestUrl} type="url" placeholder="https://example.ugm.ac.id/page" class="input" required />
    </div>
    <div class="grid sm:grid-cols-3 gap-3">
      <div>
        <label for="ingest-title" class="label">Title <span class="font-normal opacity-70">(optional)</span></label>
        <input id="ingest-title" bind:value={ingestTitle} placeholder="Auto-detected" class="input" />
      </div>
      <div>
        <label for="ingest-category" class="label">Category</label>
        <input id="ingest-category" bind:value={ingestCategory} placeholder="e.g. Academic" class="input" list="doc-categories" />
      </div>
      <div>
        <label for="ingest-subcategory" class="label">Subcategory</label>
        <input id="ingest-subcategory" bind:value={ingestSubcategory} placeholder="e.g. Curriculum" class="input" />
      </div>
    </div>
    <fieldset>
      <legend class="label">Chunking strategy</legend>
      <div class="grid grid-cols-2 sm:grid-cols-4 gap-2">
        {#each chunkStrategyOptions as opt}
          <label class="flex items-center justify-center h-10 rounded-lg border text-sm font-semibold cursor-pointer transition-colors
                        {ingestChunkStrategy === opt.value ? 'border-primary bg-primary/10 text-primary' : 'text-muted-foreground hover:border-primary/40 hover:text-foreground'}">
            <input type="radio" class="sr-only" bind:group={ingestChunkStrategy} value={opt.value} />
            {opt.label}
          </label>
        {/each}
      </div>
      <p class="text-xs text-muted-foreground mt-2">{chunkStrategyOptions.find(o => o.value === ingestChunkStrategy)?.desc}</p>
    </fieldset>
    {#if ingestError}<Alert>{ingestError}</Alert>{/if}

    {#if preview}
      <section class="rounded-lg border p-3 space-y-3" aria-label="Preview" transition:slide={{ duration: 150 }}>
        <div>
          <p class="text-sm font-semibold break-words">{preview.title}</p>
          <p class="text-xs text-muted-foreground mt-0.5">
            {preview.chunk_count} chunks · {preview.sections} sections · {preview.tables.length} tables ·
            {preview.text_chars.toLocaleString()} characters of text · strategy: {preview.strategy}
          </p>
        </div>

        {#if preview.tables.length}
          <ul class="text-xs space-y-1">
            {#each preview.tables as t}
              <li><span class="font-medium">Table{t.caption ? `: ${t.caption}` : ''}</span> — {t.rows} rows{t.headers.some(Boolean) ? ` (${t.headers.filter(Boolean).join(', ')})` : ''}</li>
            {/each}
          </ul>
        {/if}

        {#if preview.warnings.length}
          <Alert variant="warning">
            <ul class="list-disc pl-4 space-y-0.5">
              {#each preview.warnings as w}<li>{w}</li>{/each}
            </ul>
          </Alert>
        {:else}
          <Alert variant="success">No problems found.</Alert>
        {/if}

        <div class="max-h-72 overflow-y-auto space-y-1.5 pr-1">
          {#each preview.chunks as c (c.index)}
            <details class="rounded-md border bg-muted/30 text-xs">
              <summary class="cursor-pointer px-2.5 py-1.5 flex gap-2 min-w-0">
                <span class="font-mono text-muted-foreground shrink-0">#{c.index + 1}</span>
                <span class="truncate flex-1">{c.text.split('\n')[0]}</span>
                <span class="text-muted-foreground shrink-0">{c.chars} ch</span>
              </summary>
              <pre class="whitespace-pre-wrap break-words font-sans px-2.5 pb-2 leading-relaxed">{c.text}</pre>
            </details>
          {/each}
        </div>
      </section>
    {/if}
  </form>
  <svelte:fragment slot="footer">
    <button type="button" class="btn-outline" disabled={ingestLoading || previewLoading} on:click={() => (showIngest = false)}>Cancel</button>
    <button type="button" class="btn-outline" disabled={ingestLoading || previewLoading} on:click={doPreview}>
      {#if previewLoading}<Loader2 size={15} class="animate-spin" /> Reading…{:else}<Eye size={15} /> Preview{/if}
    </button>
    <button type="submit" form="ingest" class="btn-primary" disabled={ingestLoading || previewLoading}>
      {#if ingestLoading}<Loader2 size={15} class="animate-spin" /> Processing…{:else}<Globe size={15} /> {preview ? 'Save & index' : 'Fetch & index'}{/if}
    </button>
  </svelte:fragment>
</Modal>

<!-- Edit modal -->
<Modal open={!!editing} title="Edit document" on:close={() => (editing = null)}>
  <form id="edit-doc" on:submit|preventDefault={saveDoc} class="space-y-4">
    <div>
      <label for="edit-title" class="label">Title</label>
      <input id="edit-title" bind:value={editTitle} class="input" required />
    </div>
    <div class="grid sm:grid-cols-2 gap-3">
      <div>
        <label for="edit-category" class="label">Category</label>
        <input id="edit-category" bind:value={editCategory} class="input" list="doc-categories" />
      </div>
      <div>
        <label for="edit-subcategory" class="label">Subcategory</label>
        <input id="edit-subcategory" bind:value={editSubcategory} class="input" />
      </div>
    </div>
    {#if editError}<Alert>{editError}</Alert>{/if}
  </form>
  <svelte:fragment slot="footer">
    <button type="button" class="btn-outline" on:click={() => (editing = null)}>Cancel</button>
    <button type="submit" form="edit-doc" class="btn-primary" disabled={savingDoc}>
      {#if savingDoc}<Loader2 size={15} class="animate-spin" />{/if} Save
    </button>
  </svelte:fragment>
</Modal>

<datalist id="doc-categories">
  {#each categories as c}<option value={c}></option>{/each}
</datalist>

<PageHeader title="Documents" description="{documents.length} documents · {totalChunks.toLocaleString('en')} chunks indexed">
  <button on:click={refresh} class="btn-outline" disabled={loading}>
    <RefreshCw size={15} class={loading ? 'animate-spin' : ''} /> <span class="hidden sm:inline">Refresh</span>
  </button>
  <button on:click={openIngest} class="btn-primary"><Globe size={15} /> Add from URL</button>
</PageHeader>

<div class="space-y-4">
  {#if notice}<Alert variant="success" dismissible on:dismiss={() => (notice = '')}>{notice}</Alert>{/if}
  {#if error}<Alert dismissible on:dismiss={() => (error = '')}>{error}</Alert>{/if}

  <div class="flex flex-col sm:flex-row gap-3">
    <div class="relative flex-1">
      <Search size={15} class="absolute left-3 top-1/2 -translate-y-1/2 text-muted-foreground" />
      <input bind:value={search} placeholder="Search title, URL or category…" class="input pl-9" aria-label="Search documents" />
    </div>
    <select bind:value={category} class="input sm:w-56" aria-label="Filter by category">
      <option value="">All categories</option>
      {#each categories as c}<option value={c}>{c}</option>{/each}
    </select>
  </div>

  {#if loading && documents.length === 0}
    <LoadingState />
  {:else if filtered.length === 0}
    <div class="card">
      <EmptyState icon={FileText} title={documents.length ? 'No matching documents' : 'No documents yet'}
        description={documents.length ? 'Try a different search or category.' : 'Add a web page to start building the knowledge base.'} />
    </div>
  {:else}
    <ul class="card divide-y overflow-hidden">
      {#each filtered as doc (doc.document_id)}
        {@const open = expandedDoc === doc.document_id}
        <li>
          <div class="flex items-start gap-3 p-4 hover:bg-muted/30 transition-colors">
            <button on:click={() => toggleChunks(doc.document_id)} aria-expanded={open}
              class="flex-1 min-w-0 flex items-start gap-3 text-left">
              <span class="w-9 h-9 rounded-lg bg-primary/10 text-primary flex items-center justify-center shrink-0">
                <FileText size={17} />
              </span>
              <span class="flex-1 min-w-0">
                <span class="block font-semibold text-sm leading-snug line-clamp-2">{doc.title || doc.document_id}</span>
                <span class="flex flex-wrap items-center gap-1.5 mt-1.5">
                  {#if doc.category}<span class="badge-primary">{doc.category}{doc.subcategory ? ` / ${doc.subcategory}` : ''}</span>{/if}
                  <span class="badge-neutral"><Layers size={10} /> {doc.chunk_count ?? 0} chunks</span>
                  {#if doc.status === 'archived'}<span class="badge-neutral" title="Replaced by a newer version; not searchable">archived</span>{/if}
                  {#if doc.status === 'draft'}<a href="/admin/sources" class="badge-primary" title="Waiting for review in Sources">draft · review</a>{/if}
                  {#if doc.source_url}<span class="text-xs text-muted-foreground truncate max-w-[16rem]">{hostname(doc.source_url) || doc.source_url}</span>{/if}
                </span>
              </span>
              <ChevronDown size={16} class="shrink-0 mt-2 text-muted-foreground transition-transform {open ? 'rotate-180' : ''}" />
            </button>
            <div class="flex items-center shrink-0">
              {#if doc.source_url}
                <a href={doc.source_url} target="_blank" rel="noopener noreferrer" class="icon-btn hidden sm:inline-flex" title="Open source" aria-label="Open source"><ExternalLink size={15} /></a>
              {/if}
              <button on:click={() => startEdit(doc)} class="icon-btn" title="Edit" aria-label="Edit document"><Pencil size={15} /></button>
              <button on:click={() => deleteDoc(doc)} class="icon-btn hover:!text-destructive hover:!bg-destructive/10" title="Delete" aria-label="Delete document"><Trash2 size={15} /></button>
            </div>
          </div>

          {#if open}
            <div class="px-4 pb-4 sm:pl-16" transition:slide={{ duration: 150 }}>
              {#if chunksLoading[doc.document_id]}
                <p class="flex items-center gap-2 text-sm text-muted-foreground"><Loader2 size={14} class="animate-spin" /> Loading chunks…</p>
              {:else if chunksError[doc.document_id]}
                <Alert>{chunksError[doc.document_id]}</Alert>
              {:else if (chunksByDoc[doc.document_id] ?? []).length === 0}
                <p class="text-sm text-muted-foreground">No chunks.</p>
              {:else}
                <div class="space-y-2 max-h-[28rem] overflow-y-auto pr-1">
                  {#each chunksByDoc[doc.document_id] as chunk (chunk.id)}
                    <div class="rounded-lg border bg-muted/30 p-3">
                      <p class="text-[11px] font-mono text-muted-foreground mb-1">#{chunk.chunk_index} · id {chunk.id}</p>
                      <p class="text-xs leading-relaxed line-clamp-5 whitespace-pre-wrap">{chunk.content}</p>
                    </div>
                  {/each}
                </div>
              {/if}
            </div>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
</div>

<script lang="ts">
  import { onMount } from 'svelte';
  import { slide } from 'svelte/transition';
  import { admin } from '$lib/api/client';
  import type { AdminDocument, AdminChunk } from '$lib/api/client';
  import { errorMessage, hostname } from '$lib/format';
  import PageHeader from '$lib/components/ui/PageHeader.svelte';
  import Alert from '$lib/components/ui/Alert.svelte';
  import Modal from '$lib/components/ui/Modal.svelte';
  import LoadingState from '$lib/components/ui/LoadingState.svelte';
  import EmptyState from '$lib/components/ui/EmptyState.svelte';
  import { Loader2, Trash2, RefreshCw, ChevronDown, Pencil, Search, FileText, ExternalLink, Layers, FolderSync, Network } from 'lucide-svelte';

  let documents: AdminDocument[] = [];
  let loading = false;
  let error = '';
  let notice = '';
  let search = '';
  let category = '';

  // Chunks
  let expandedDoc: string | null = null;
  let chunksByDoc: Record<string, AdminChunk[]> = {};
  let chunksLoading: Record<string, boolean> = {};
  let chunksError: Record<string, string> = {};

  // Edit
  let editing: AdminDocument | null = null;
  let editTitle = ''; let editCategory = ''; let editSubcategory = '';
  let editVersion = ''; let editValidFrom = '';
  let savingDoc = false; let editError = '';

  // Source groups the admin opened or closed (kept in this browser only)
  const OPEN_KEY = 'admin-docs-open-groups';
  let openGroups: Record<string, boolean> = {};
  onMount(() => {
    try { openGroups = JSON.parse(localStorage.getItem(OPEN_KEY) || '{}'); } catch { openGroups = {}; }
    loadDocs();
  });
  function saveOpen() {
    try { localStorage.setItem(OPEN_KEY, JSON.stringify(openGroups)); } catch { /* storage unavailable */ }
  }
  /** Small groups and groups with drafts start open; a search or filter opens every match. */
  function isOpen(g: DocGroup, openGroups: Record<string, boolean>, filtering: boolean): boolean {
    if (filtering) return true;
    return openGroups[g.key] ?? (g.docs.length <= 5 || reviewCount(g) > 0);
  }
  function toggleGroup(g: DocGroup) {
    openGroups = { ...openGroups, [g.key]: !isOpen(g, openGroups, false) };
    saveOpen();
  }
  function setAll(open: boolean) {
    openGroups = Object.fromEntries(groups.map(g => [g.key, open]));
    saveOpen();
  }

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
    editVersion = doc.version_label ?? '';
    editValidFrom = doc.valid_from ?? '';
    editError = '';
  }

  async function saveDoc() {
    if (!editing) return;
    const id = editing.document_id;
    const patch = {
      title: editTitle.trim(), category: editCategory.trim(), subcategory: editSubcategory.trim(),
      version_label: editVersion.trim(), valid_from: editValidFrom || ''
    };
    savingDoc = true; editError = '';
    try {
      await admin.updateDocument(id, patch);
      documents = documents.map(d => (d.document_id === id ? { ...d, ...patch, valid_from: patch.valid_from || null } : d));
      const { [id]: _, ...rest } = chunksByDoc;
      chunksByDoc = rest;
      editing = null;
    } catch (e) { editError = errorMessage(e); }
    finally { savingDoc = false; }
  }

  $: categories = [...new Set(documents.map(d => d.category).filter(Boolean))].sort();
  $: totalChunks = documents.filter(d => (d.status ?? 'active') === 'active').reduce((n, d) => n + (d.chunk_count ?? 0), 0);
  $: activeCount = documents.filter(d => (d.status ?? 'active') === 'active').length;

  // One group per source (a website with its pages, a Drive folder with its PDFs);
  // documents added before sources existed are grouped as "Added manually".
  interface DocGroup { key: string; title: string; kind: string; root: string; docs: AdminDocument[] }
  $: groups = filtered.reduce<DocGroup[]>((acc, d) => {
    const key = d.source_id ? `src-${d.source_id}` : 'manual';
    let g = acc.find(x => x.key === key);
    if (!g) {
      g = d.source_id
        ? { key, title: d.source_title || d.source_root || 'Source', kind: d.source_kind ?? '', root: d.source_root ?? '', docs: [] }
        : { key, title: 'Added manually (before Sources)', kind: '', root: '', docs: [] };
      acc.push(g);
    }
    g.docs.push(d);
    return acc;
  }, []).map(g => ({ ...g, docs: [...g.docs].sort(byStatus) })).sort((a, b) => (a.key === 'manual' ? 1 : b.key === 'manual' ? -1 : a.title.localeCompare(b.title)));
  $: q = search.trim().toLowerCase();
  /** '' = active and in review; 'all' adds archived */
  let statusFilter: '' | 'active' | 'draft' | 'archived' | 'all' = '';
  $: filtering = !!q || !!category || (statusFilter !== '' && statusFilter !== 'all');
  const STATUS_ORDER: Record<string, number> = { draft: 0, active: 1, archived: 2 };
  function byStatus(a: AdminDocument, b: AdminDocument): number {
    return (STATUS_ORDER[a.status ?? 'active'] - STATUS_ORDER[b.status ?? 'active'])
      || (a.title || a.document_id).localeCompare(b.title || b.document_id, 'id', { numeric: true });
  }
  function archivedIn(g: DocGroup): number {
    return new Set(g.docs.filter(d => d.status === 'archived').map(d => d.source_url || d.document_id)).size;
  }
  /** the version most documents in the group carry, shown once on the group header */
  function groupVersion(g: DocGroup): string {
    const counts = new Map<string, number>();
    for (const d of g.docs) if (d.status !== 'archived') { const v = versionOf(d); if (v) counts.set(v, (counts.get(v) ?? 0) + 1); }
    return [...counts.entries()].sort((a, b) => b[1] - a[1])[0]?.[0] ?? '';
  }
  function reviewCount(g: DocGroup): number {
    return new Set(g.docs.filter(d => d.status === 'draft').map(d => d.source_url || d.document_id)).size;
  }
  /** active documents in a group, one per address (versions and drafts are not counted twice) */
  function liveCount(g: DocGroup): number {
    return new Set(g.docs.filter(d => d.status === 'active').map(d => d.source_url || d.document_id)).size;
  }
  /** "2022 Revisi-2 · 2025-11-01": the document's own version, else its source's */
  function versionOf(d: AdminDocument): string {
    const own = !!(d.version_label || d.valid_from);   // label and date always as one pair
    const label = own ? d.version_label : d.source_version;
    const date = own ? d.valid_from : d.source_valid_from;
    return [label, date].filter(Boolean).join(' · ');
  }
  $: archivedCount = documents.filter(d => d.status === 'archived').length;
  $: filtered = documents.filter(d =>
    (statusFilter === 'all' ? true
      : statusFilter === '' ? d.status !== 'archived'
      : (d.status ?? 'active') === statusFilter) &&
    (!category || d.category === category) &&
    (!q || [d.title, d.source_url, d.category, d.subcategory].some(v => v?.toLowerCase().includes(q)))
  );
</script>

<svelte:head><title>Documents · Admin</title></svelte:head>

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
    <div class="grid sm:grid-cols-2 gap-3">
      <div>
        <label for="edit-version" class="label">Version</label>
        <input id="edit-version" bind:value={editVersion} class="input"
          placeholder={editing?.source_version || 'e.g. 2022 Revisi-2'} />
      </div>
      <div>
        <label for="edit-valid" class="label">Valid from</label>
        <input id="edit-valid" type="date" bind:value={editValidFrom} class="input" />
      </div>
    </div>
    <p class="text-xs text-muted-foreground -mt-2">
      When documents disagree (e.g. SKS totals), answers follow the newest “valid from”.
      {#if editing?.source_id}Leave empty to use the source's version{editing?.source_version ? ` (${editing.source_version})` : ''}.{/if}
    </p>
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

<PageHeader title="Documents" description="{activeCount} documents · {totalChunks.toLocaleString('en')} chunks used in answers. Add or update content in Sources.">
  <button on:click={refresh} class="btn-outline" disabled={loading}>
    <RefreshCw size={15} class={loading ? 'animate-spin' : ''} /> <span class="hidden sm:inline">Refresh</span>
  </button>
  <a href="/admin/sources" class="btn-primary"><FolderSync size={15} /> Add in Sources</a>
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
    <select bind:value={statusFilter} class="input sm:w-48" aria-label="Filter by status">
      <option value="">Active + in review</option>
      <option value="active">Active only</option>
      <option value="draft">In review</option>
      <option value="archived">Archived ({archivedCount})</option>
      <option value="all">All</option>
    </select>
  </div>

  {#if groups.length > 1 && !filtering}
    <div class="flex justify-end gap-3 -mt-1 text-xs">
      <button type="button" class="text-muted-foreground hover:text-foreground" on:click={() => setAll(true)}>Expand all</button>
      <button type="button" class="text-muted-foreground hover:text-foreground" on:click={() => setAll(false)}>Collapse all</button>
    </div>
  {/if}

  {#if loading && documents.length === 0}
    <LoadingState />
  {:else if filtered.length === 0}
    <div class="card">
      <EmptyState icon={FileText} title={documents.length ? 'No matching documents' : 'No documents yet'}
        description={documents.length ? 'Try a different search or category.' : 'Add sources (web pages, PDFs, Drive) in Admin → Sources.'} />
    </div>
  {:else}
    {#each groups as g (g.key)}
    {@const gOpen = isOpen(g, openGroups, filtering)}
    {@const gVersion = groupVersion(g)}
    <section class="card overflow-hidden" aria-label={g.title}>
      <div class="flex items-center gap-2 px-4 py-3 min-w-0 {gOpen ? 'border-b' : ''}">
        <button type="button" on:click={() => toggleGroup(g)} aria-expanded={gOpen}
          class="flex-1 min-w-0 flex items-center gap-2 text-left">
          <ChevronDown size={16} class="shrink-0 text-muted-foreground transition-transform {gOpen ? '' : '-rotate-90'}" />
          <svelte:component this={g.kind === 'web_site' ? Network : g.key === 'manual' ? FileText : FolderSync} size={15} class="text-muted-foreground shrink-0" />
          <span class="text-sm font-semibold truncate">{g.title}</span>
          <span class="flex flex-wrap items-center gap-1.5 shrink-0">
            <span class="badge-neutral">{liveCount(g)} {g.kind === 'web_site' ? (liveCount(g) === 1 ? 'page' : 'pages') : liveCount(g) === 1 ? 'document' : 'documents'}</span>
            {#if reviewCount(g)}<span class="badge-primary">{reviewCount(g)} in review</span>{/if}
            {#if archivedIn(g)}<span class="badge-neutral hidden sm:inline-flex">{archivedIn(g)} archived</span>{/if}
            {#if gVersion}<span class="badge-neutral hidden md:inline-flex" title="Version used when sources disagree">{gVersion}</span>{/if}
          </span>
        </button>
        {#if g.root}<a href={g.root} target="_blank" rel="noopener noreferrer" class="icon-btn hidden sm:inline-flex shrink-0" title={g.root} aria-label="Open source site"><ExternalLink size={15} /></a>{/if}
      </div>
    {#if gOpen}
    <ul class="divide-y" transition:slide={{ duration: 150 }}>
      {#each g.docs as doc (doc.document_id)}
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
                  {#if versionOf(doc) && versionOf(doc) !== gVersion}<span class="badge-neutral" title="Version used when sources disagree">{versionOf(doc)}</span>{/if}
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
    </section>
    {/each}
  {/if}
</div>

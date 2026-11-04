<script lang="ts">
  import { onMount } from 'svelte';
  import { slide } from 'svelte/transition';
  import { admin } from '$lib/api/client';
  import type { AdminDocument, AdminChunk, Category } from '$lib/api/client';
  import { errorMessage, hostname } from '$lib/format';
  import PageHeader from '$lib/components/ui/PageHeader.svelte';
  import Alert from '$lib/components/ui/Alert.svelte';
  import Modal from '$lib/components/ui/Modal.svelte';
  import LoadingState from '$lib/components/ui/LoadingState.svelte';
  import EmptyState from '$lib/components/ui/EmptyState.svelte';
  import CategoryTree from '$lib/components/admin/CategoryTree.svelte';
  import type { CategoryNode } from '$lib/components/admin/CategoryTree.svelte';
  import { Loader2, Trash2, RefreshCw, ChevronDown, Pencil, Search, FileText, ExternalLink, Layers, FolderSync, Network, Folder, FolderInput, Tag } from 'lucide-svelte';

  let documents: AdminDocument[] = [];
  let loading = false;
  let error = '';
  let notice = '';
  let search = '';

  // Topic categories (Admin → Documents). Selected tree entry:
  // '' = all documents, 'none' = uncategorized, otherwise a category id as a string.
  let categoryList: Category[] = [];
  let apiUncategorized = 0;
  let selectedCat = '';

  // Chunks
  let expandedDoc: string | null = null;
  let chunksByDoc: Record<string, AdminChunk[]> = {};
  let chunksLoading: Record<string, boolean> = {};
  let chunksError: Record<string, string> = {};

  // Edit
  let editing: AdminDocument | null = null;
  let editTitle = ''; let editCategory = ''; let editSubcategory = '';
  let editVersion = ''; let editValidFrom = '';
  let editCategoryId = 'none';
  let savingDoc = false; let editError = '';

  // Bulk move
  let selection = new Set<string>();
  let moveTarget = '';
  let moving = false;

  // Source groups the admin opened or closed (kept in this browser only)
  const OPEN_KEY = 'admin-docs-open-groups';
  let openGroups: Record<string, boolean> = {};
  onMount(() => {
    try { openGroups = JSON.parse(localStorage.getItem(OPEN_KEY) || '{}'); } catch { openGroups = {}; }
    loadDocs();
    loadCategories();
  });
  function saveOpen() {
    try { localStorage.setItem(OPEN_KEY, JSON.stringify(openGroups)); } catch { /* storage unavailable */ }
  }
  /** Small groups and groups with drafts start open; a search or filter opens every match. */
  function isOpen(g: DocGroup, openGroups: Record<string, boolean>, filtering: boolean): boolean {
    if (filtering || g.key === FLAT) return true;
    return openGroups[g.key] ?? (g.docs.length <= 5 || reviewCount(g) > 0);
  }
  function toggleGroup(g: DocGroup) {
    if (g.key === FLAT) return;
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

  async function loadCategories() {
    try {
      const res = await admin.categories();
      categoryList = res.categories ?? [];
      apiUncategorized = res.uncategorized ?? 0;
    } catch (e) { error = `Could not load categories: ${errorMessage(e)}`; }
  }

  function refresh() {
    chunksByDoc = {};
    loadDocs();
    loadCategories();
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
      if (selection.has(doc.document_id)) { selection.delete(doc.document_id); selection = selection; }
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
    editCategoryId = doc.category_id != null ? String(doc.category_id) : 'none';
    editError = '';
  }

  /** '' / 'none' → null, '7' → 7 */
  function toCategoryId(key: string): number | null {
    return key && key !== 'none' ? Number(key) : null;
  }

  async function saveDoc() {
    if (!editing) return;
    const id = editing.document_id;
    const patch = {
      title: editTitle.trim(), category: editCategory.trim(), subcategory: editSubcategory.trim(),
      version_label: editVersion.trim(), valid_from: editValidFrom || ''
    };
    const newCategoryId = toCategoryId(editCategoryId);
    const categoryChanged = newCategoryId !== (editing.category_id ?? null);
    savingDoc = true; editError = '';
    try {
      await admin.updateDocument(id, patch);
      documents = documents.map(d => (d.document_id === id ? { ...d, ...patch, valid_from: patch.valid_from || null } : d));
      const { [id]: _, ...rest } = chunksByDoc;
      chunksByDoc = rest;
      if (categoryChanged) {
        await admin.setDocumentCategory(id, newCategoryId);
        documents = documents.map(d => (d.document_id === id
          ? { ...d, category_id: newCategoryId, category_path: pathOf(newCategoryId) } : d));
        loadCategories();
      }
      editing = null;
    } catch (e) { editError = errorMessage(e); }
    finally { savingDoc = false; }
  }

  // ── Bulk selection and move ──
  function toggleSelect(id: string) {
    if (selection.has(id)) selection.delete(id); else selection.add(id);
    selection = selection;
  }
  function clearSelection() { selection = new Set(); }
  /** `indeterminate` is a DOM property only (no HTML attribute), so set it from an action. */
  function indeterminate(node: HTMLInputElement, value: boolean) {
    node.indeterminate = value;
    return { update(v: boolean) { node.indeterminate = v; } };
  }
  function toggleSelectAll() {
    selection = allVisibleSelected ? new Set() : new Set(filtered.map(d => d.document_id));
  }

  async function moveSelected() {
    if (!moveTarget || selection.size === 0) return;
    const target = toCategoryId(moveTarget);
    const ids = [...selection];
    moving = true; error = ''; notice = '';
    let ok = 0;
    const failures: string[] = [];
    const BATCH = 4;
    for (let i = 0; i < ids.length; i += BATCH) {
      const batch = ids.slice(i, i + BATCH);
      const results = await Promise.allSettled(batch.map(id => admin.setDocumentCategory(id, target)));
      results.forEach((r, j) => {
        if (r.status === 'fulfilled') ok += 1;
        else failures.push(`${titleOf(batch[j])}: ${errorMessage(r.reason)}`);
      });
    }
    const where = target === null ? 'Uncategorized' : pathOf(target);
    if (ok) notice = `Moved ${ok} ${ok === 1 ? 'document' : 'documents'} to ${where}.`;
    if (failures.length) {
      error = `${failures.length} ${failures.length === 1 ? 'document' : 'documents'} could not be moved — ${failures.slice(0, 3).join('; ')}${failures.length > 3 ? '; …' : ''}`;
    }
    selection = new Set();
    moveTarget = '';
    moving = false;
    await Promise.all([loadDocs(), loadCategories()]);
  }

  function titleOf(id: string): string {
    const d = documents.find(x => x.document_id === id);
    return d?.title || id;
  }

  // ── Category tree ──
  $: categoryById = new Map(categoryList.map(c => [c.id, c]));
  /** "Akademik › Persuratan"; '' for null or an unknown id */
  function pathOf(id: number | null): string {
    if (id == null) return '';
    const c = categoryById.get(id);
    if (!c) return '';
    const parent = c.parent_id != null ? categoryById.get(c.parent_id) : undefined;
    return parent ? `${parent.name} › ${c.name}` : c.name;
  }
  $: parents = categoryList.filter(c => c.parent_id == null);
  $: childrenOf = categoryList.reduce<Map<number, Category[]>>((m, c) => {
    if (c.parent_id != null) m.set(c.parent_id, [...(m.get(c.parent_id) ?? []), c]);
    return m;
  }, new Map());

  // Counts follow the selected status tab, computed from the loaded documents so they always
  // match the list. Until documents are loaded, the API's counts (archived excluded) are used.
  $: statusDocs = documents.filter(d => (d.status ?? 'active') === statusFilter);
  $: countsFromDocs = documents.length > 0 || !categoryList.length;
  $: directCounts = countsFromDocs
    ? statusDocs.reduce<Map<number, number>>((m, d) => {
        if (d.category_id != null) m.set(d.category_id, (m.get(d.category_id) ?? 0) + 1);
        return m;
      }, new Map())
    : new Map(categoryList.map(c => [c.id, c.document_count]));
  $: uncategorizedCount = countsFromDocs ? statusDocs.filter(d => d.category_id == null).length : apiUncategorized;
  $: allCount = countsFromDocs ? statusDocs.length
    : categoryList.reduce((n, c) => n + c.document_count, 0) + apiUncategorized;
  $: treeNodes = parents.map<CategoryNode>(p => {
    const children = (childrenOf.get(p.id) ?? []).map(c => ({ cat: c, count: directCounts.get(c.id) ?? 0 }));
    const count = directCounts.get(p.id) ?? 0;
    return { cat: p, count, total: count + children.reduce((n, c) => n + c.count, 0), children };
  });
  /** select options: parents, then their children indented */
  $: categoryOptions = parents.flatMap(p => [
    { value: String(p.id), label: p.name, depth: 0 },
    ...(childrenOf.get(p.id) ?? []).map(c => ({ value: String(c.id), label: c.name, depth: 1 }))
  ]);
  const INDENT = '    ';

  /** ids a tree entry covers: a parent covers its children too; null = no category filter */
  $: selectedIds = selectedCat && selectedCat !== 'none'
    ? new Set([Number(selectedCat), ...(childrenOf.get(Number(selectedCat)) ?? []).map(c => c.id)])
    : null;
  $: selectedLabel = categoryById && selectedCat === 'none' ? 'Uncategorized' : selectedCat ? pathOf(Number(selectedCat)) || 'Category' : 'All documents';
  function inSelectedCategory(d: AdminDocument, selectedCat: string, selectedIds: Set<number> | null): boolean {
    if (!selectedCat) return true;
    if (selectedCat === 'none') return d.category_id == null;
    return d.category_id != null && !!selectedIds?.has(d.category_id);
  }

  // A new tab or category shows other documents: start the selection over.
  $: { statusFilter; selectedCat; selection = new Set(); }

  $: categories = [...new Set(documents.map(d => d.category).filter(Boolean))].sort();
  $: totalChunks = documents.filter(d => (d.status ?? 'active') === 'active').reduce((n, d) => n + (d.chunk_count ?? 0), 0);
  $: activeCount = documents.filter(d => (d.status ?? 'active') === 'active').length;

  // One group per source (a website with its pages, a Drive folder with its PDFs);
  // documents added before sources existed are grouped as "Added manually".
  // With a category selected the list is flat (one pseudo-group, sorted by title).
  interface DocGroup { key: string; title: string; kind: string; root: string; docs: AdminDocument[] }
  const FLAT = 'flat';
  $: groups = selectedCat
    ? (filtered.length ? [{ key: FLAT, title: selectedLabel, kind: '', root: '', docs: [...filtered].sort(byStatus) }] : [])
    : filtered.reduce<DocGroup[]>((acc, d) => {
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
  /** Status tab. Active = what answers use (the default); drafts are reviewed in Sources. */
  type StatusTab = 'active' | 'draft' | 'archived';
  let statusFilter: StatusTab = 'active';
  const STATUS_TABS: { key: StatusTab; label: string }[] = [
    { key: 'active', label: 'Active' }, { key: 'draft', label: 'In review' }, { key: 'archived', label: 'Archived' }
  ];
  $: statusCounts = documents.reduce<Record<StatusTab, number>>((n, d) => {
    n[(d.status ?? 'active') as StatusTab] += 1;
    return n;
  }, { active: 0, draft: 0, archived: 0 });
  $: filtering = !!q || !!selectedCat || statusFilter !== 'active';
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
    if (g.key === FLAT) return '';   // documents from different sources: show each one's version
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
  $: filtered = documents.filter(d =>
    (d.status ?? 'active') === statusFilter &&
    inSelectedCategory(d, selectedCat, selectedIds) &&
    (!q || [d.title, d.source_url, d.category, d.subcategory, d.category_path].some(v => v?.toLowerCase().includes(q)))
  );
  $: allVisibleSelected = filtered.length > 0 && filtered.every(d => selection.has(d.document_id));
  $: someVisibleSelected = filtered.some(d => selection.has(d.document_id));
</script>

<svelte:head><title>Documents · Admin</title></svelte:head>

<!-- Edit modal -->
<Modal open={!!editing} title="Edit document" on:close={() => (editing = null)}>
  <form id="edit-doc" on:submit|preventDefault={saveDoc} class="space-y-4">
    <div>
      <label for="edit-title" class="label">Title</label>
      <input id="edit-title" bind:value={editTitle} class="input" required />
    </div>
    <div>
      <label for="edit-topic" class="label">Category</label>
      <select id="edit-topic" bind:value={editCategoryId} class="input">
        <option value="none">Uncategorized</option>
        {#each categoryOptions as o (o.value)}
          <option value={o.value}>{o.depth ? INDENT : ''}{o.label}</option>
        {/each}
      </select>
      <p class="text-xs text-muted-foreground mt-1.5">Where the document is filed in this admin list.</p>
    </div>
    <div class="grid sm:grid-cols-2 gap-3">
      <div>
        <label for="edit-category" class="label">Label (shown in chat source cards)</label>
        <input id="edit-category" bind:value={editCategory} class="input" list="doc-categories" />
      </div>
      <div>
        <label for="edit-subcategory" class="label">Sub-label (shown in chat source cards)</label>
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

  <div class="flex flex-wrap items-center gap-x-4 gap-y-2">
    <div role="tablist" aria-label="Document status" class="inline-flex rounded-lg border bg-muted/40 p-0.5">
      {#each STATUS_TABS as t}
        <button type="button" role="tab" aria-selected={statusFilter === t.key} on:click={() => (statusFilter = t.key)}
          class="px-3 py-1.5 rounded-md text-sm font-medium transition-colors
                 {statusFilter === t.key ? 'bg-card text-foreground shadow-sm' : 'text-muted-foreground hover:text-foreground'}">
          {t.label} <span class="ml-1 text-xs tabular-nums {statusFilter === t.key ? 'text-muted-foreground' : 'opacity-70'}">{statusCounts[t.key]}</span>
        </button>
      {/each}
    </div>
    <p class="text-xs text-muted-foreground">
      {#if statusFilter === 'active'}Used in answers.
      {:else if statusFilter === 'draft'}Not used in answers yet. <a href="/admin/sources" class="text-primary hover:underline">Review drafts in Sources</a>
      {:else}Replaced by a newer version; kept for the record, not used in answers.{/if}
    </p>
  </div>

  <div class="lg:grid lg:grid-cols-[15rem_minmax(0,1fr)] lg:gap-6 lg:items-start">
    <!-- Category tree (desktop) -->
    <aside class="hidden lg:block card p-2 sticky top-4 max-h-[calc(100vh-8rem)] overflow-y-auto">
      <CategoryTree bind:selected={selectedCat} nodes={treeNodes} {allCount} {uncategorizedCount} />
    </aside>

    <div class="space-y-4 min-w-0">
      <div class="flex flex-col sm:flex-row gap-3">
        <!-- Category picker (mobile) -->
        <select bind:value={selectedCat} class="input lg:hidden sm:w-64" aria-label="Filter by category">
          <option value="">All documents ({allCount})</option>
          {#each treeNodes as node (node.cat.id)}
            {#if node.children.length}
              <optgroup label={node.cat.name}>
                <option value={String(node.cat.id)}>All in {node.cat.name} ({node.total})</option>
                {#each node.children as child (child.cat.id)}
                  <option value={String(child.cat.id)}>{child.cat.name} ({child.count})</option>
                {/each}
              </optgroup>
            {:else}
              <option value={String(node.cat.id)}>{node.cat.name} ({node.total})</option>
            {/if}
          {/each}
          <option value="none">⚠ Uncategorized ({uncategorizedCount})</option>
        </select>
        <div class="relative flex-1">
          <Search size={15} class="absolute left-3 top-1/2 -translate-y-1/2 text-muted-foreground" />
          <input bind:value={search} placeholder={selectedCat ? `Search in ${selectedLabel}…` : 'Search title, URL or category…'}
            class="input pl-9" aria-label="Search documents" />
        </div>
      </div>

      {#if filtered.length > 0}
        <div class="flex flex-wrap items-center gap-x-4 gap-y-2 text-xs">
          <label class="inline-flex items-center gap-2 text-muted-foreground hover:text-foreground cursor-pointer">
            <input type="checkbox" class="w-4 h-4 rounded accent-[hsl(var(--primary))]"
              checked={allVisibleSelected} use:indeterminate={someVisibleSelected && !allVisibleSelected}
              on:change={toggleSelectAll} />
            Select all {filtered.length} shown
          </label>
          {#if selectedCat}
            <span class="text-muted-foreground">{selectedLabel}</span>
          {/if}
          {#if groups.length > 1 && !filtering}
            <div class="ml-auto flex gap-3">
              <button type="button" class="text-muted-foreground hover:text-foreground" on:click={() => setAll(true)}>Expand all</button>
              <button type="button" class="text-muted-foreground hover:text-foreground" on:click={() => setAll(false)}>Collapse all</button>
            </div>
          {/if}
        </div>
      {/if}

      {#if loading && documents.length === 0}
        <LoadingState />
      {:else if filtered.length === 0}
        <div class="card">
          <EmptyState icon={selectedCat ? Folder : FileText}
            title={!documents.length ? 'No documents yet'
              : q ? 'No matching documents'
              : selectedCat === 'none' && statusFilter === 'active' ? 'Every document has a category'
              : selectedCat ? 'No documents in this category yet'
              : statusFilter === 'draft' ? 'Nothing waiting for review' : statusFilter === 'archived' ? 'No archived documents' : 'No active documents'}
            description={!documents.length ? 'Add sources (web pages, PDFs, Drive) in Admin → Sources.'
              : q ? 'Try a different search or category.'
              : selectedCat && selectedCat !== 'none' ? 'Select documents in another category and use “Move to…” to file them here.'
              : ''} />
        </div>
      {:else}
        {#each groups as g (g.key)}
        {@const gOpen = isOpen(g, openGroups, filtering)}
        {@const gVersion = groupVersion(g)}
        <section class="card overflow-hidden" aria-label={g.title}>
          {#if g.key === FLAT}
            <div class="flex items-center gap-2 px-4 py-3 min-w-0 border-b">
              <svelte:component this={selectedCat === 'none' ? Tag : Folder} size={15} class="text-muted-foreground shrink-0" />
              <span class="text-sm font-semibold truncate">{g.title}</span>
              <span class="badge-neutral shrink-0">{g.docs.length} {g.docs.length === 1 ? 'document' : 'documents'}</span>
            </div>
          {:else}
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
          {/if}
        {#if gOpen}
        <ul class="divide-y" transition:slide={{ duration: 150 }}>
          {#each g.docs as doc (doc.document_id)}
            {@const open = expandedDoc === doc.document_id}
            {@const checked = selection.has(doc.document_id)}
            <li class={checked ? 'bg-primary/5' : ''}>
              <div class="flex items-start gap-3 p-4 hover:bg-muted/30 transition-colors">
                <input type="checkbox" class="mt-2.5 w-4 h-4 rounded shrink-0 accent-[hsl(var(--primary))] cursor-pointer"
                  {checked} on:change={() => toggleSelect(doc.document_id)}
                  aria-label="Select {doc.title || doc.document_id}" />
                <button on:click={() => toggleChunks(doc.document_id)} aria-expanded={open}
                  class="flex-1 min-w-0 flex items-start gap-3 text-left">
                  <span class="w-9 h-9 rounded-lg bg-primary/10 text-primary hidden sm:flex items-center justify-center shrink-0">
                    <FileText size={17} />
                  </span>
                  <span class="flex-1 min-w-0">
                    <span class="block font-semibold text-sm leading-snug line-clamp-2">{doc.title || doc.document_id}</span>
                    <span class="flex flex-wrap items-center gap-1.5 mt-1.5">
                      {#if doc.category_path}
                        <span class="badge-primary max-w-full" title="Category"><Folder size={10} class="shrink-0" /> <span class="truncate">{doc.category_path}</span></span>
                      {:else}
                        <span class="badge-warning" title="Not filed under a category yet">Uncategorized</span>
                      {/if}
                      {#if doc.category}<span class="badge-neutral hidden sm:inline-flex" title="Label shown in chat source cards"><Tag size={10} /> {doc.category}{doc.subcategory ? ` / ${doc.subcategory}` : ''}</span>{/if}
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

      <!-- Bulk action bar: sticks to the bottom of the scrolling admin pane -->
      {#if selection.size > 0}
        <div class="sticky bottom-4 z-20 card shadow-lg px-3 py-2.5 flex flex-wrap items-center gap-2"
          role="region" aria-label="Bulk actions" transition:slide={{ duration: 150 }}>
          <span class="text-sm font-semibold tabular-nums">{selection.size} selected</span>
          <span class="text-muted-foreground hidden sm:inline" aria-hidden="true">·</span>
          <form class="flex flex-1 min-w-0 flex-wrap sm:flex-nowrap items-center gap-2" on:submit|preventDefault={moveSelected}>
            <label for="move-target" class="sr-only">Move selected documents to</label>
            <select id="move-target" bind:value={moveTarget} class="input h-9 flex-1 min-w-0 sm:max-w-xs" disabled={moving}>
              <option value="" disabled>Move to…</option>
              {#each categoryOptions as o (o.value)}
                <option value={o.value}>{o.depth ? INDENT : ''}{o.label}</option>
              {/each}
              <option value="none">Uncategorized</option>
            </select>
            <button type="submit" class="btn-primary btn-sm" disabled={!moveTarget || moving}>
              {#if moving}<Loader2 size={14} class="animate-spin" />{:else}<FolderInput size={14} />{/if} Move
            </button>
            <button type="button" class="btn-ghost btn-sm" on:click={clearSelection} disabled={moving}>Clear</button>
          </form>
        </div>
      {/if}
    </div>
  </div>
</div>

<script lang="ts">
  import { onMount } from 'svelte';
  import { slide } from 'svelte/transition';
  import { admin } from '$lib/api/client';
  import type {
    SourceRow, DraftDocument, DraftChunk, IngestJob, NewSourcePayload, ResolvedLink, SourceFile, SourceUpdatePayload
  } from '$lib/api/client';
  import { errorMessage, formatDateTime, hostname } from '$lib/format';
  import PageHeader from '$lib/components/ui/PageHeader.svelte';
  import Alert from '$lib/components/ui/Alert.svelte';
  import Modal from '$lib/components/ui/Modal.svelte';
  import LoadingState from '$lib/components/ui/LoadingState.svelte';
  import EmptyState from '$lib/components/ui/EmptyState.svelte';
  import {
    Loader2, RefreshCw, Plus, Trash2, ExternalLink, FolderOpen, FileText, Globe, Check, X,
    ChevronDown, Clock, AlertTriangle, CircleCheck, Image, Table2, FolderSync, Network, Search, ArrowLeft, Pencil
  } from 'lucide-svelte';

  let sources: SourceRow[] = [];
  let drafts: DraftDocument[] = [];
  let loading = false;
  let error = '';
  let notice = '';

  // ── Add source: step 1 = links, step 2 = what they are + details ──
  let showAdd = false;
  let linksText = '';
  let resolved: ResolvedLink | null = null;
  let resolving = false;
  /** results of "Check links", split by what each link is */
  let pages: ResolvedLink[] = [];
  let others: ResolvedLink[] = [];
  let known: ResolvedLink[] = [];
  let sameParent = false;
  /** web pages: 'site' = part of a site source, 'parent' = add to an existing site, 'single' = standalone pages */
  let pageMode: 'site' | 'parent' | 'single' = 'site';
  let siteRoot = '';
  let discover: 'manual' | 'auto' = 'manual';
  let form: NewSourcePayload = blankForm();
  let adding = false;
  let addError = '';

  // ── Review ──
  let openDraft: string | null = null;
  let draftChunks: Record<string, DraftChunk[]> = {};
  let busy: Record<string, boolean> = {};

  // ── Sources: history and pages ──
  let openJobs: number | null = null;
  let jobs: Record<number, IngestJob[]> = {};
  let openFiles: number | null = null;
  let files: Record<number, SourceFile[]> = {};
  let newPages: Record<number, string> = {};

  const kindInfo: Record<string, { label: string; icon: typeof Globe }> = {
    drive_folder: { label: 'Drive folder', icon: FolderOpen },
    drive_file:   { label: 'Drive file',   icon: FileText },
    web_pdf:      { label: 'Website PDF',  icon: FileText },
    web_site:     { label: 'Website',      icon: Network },
    web_page:     { label: 'Web page',     icon: Globe }
  };
  const fileStatus: Record<string, string> = {
    active: 'badge-success', draft: 'badge-primary', pending: 'badge-neutral', rejected: 'badge-neutral',
    missing: 'badge-warning', duplicate: 'badge-neutral', error: 'badge-danger', archived: 'badge-neutral'
  };
  const stateStyle: Record<string, string> = {
    queued: 'badge-neutral', running: 'badge-primary', done: 'badge-success', failed: 'badge-danger'
  };

  function blankForm(): NewSourcePayload {
    return { url: '', title: '', category: '', subcategory: '', version_label: '', valid_from: '', supersedes_id: null, auto_check: true };
  }

  onMount(load);

  async function load() {
    loading = true; error = '';
    try {
      [sources, drafts] = await Promise.all([admin.sources(), admin.drafts()]);
      if (openFiles !== null) files = { ...files, [openFiles]: await admin.sourceFiles(openFiles) };
    } catch (e) {
      error = errorMessage(e);
    } finally {
      loading = false;
    }
  }

  // ── Add flow ──
  $: links = linksText.split(/\s+/).map(l => l.trim()).filter(Boolean);

  function openAdd() {
    linksText = ''; resolved = null; pages = []; others = []; known = []; addError = ''; form = blankForm();
    pageMode = 'site'; siteRoot = ''; discover = 'manual';
    showAdd = true;
  }

  async function checkLinks() {
    if (!links.length) return;
    resolving = true; addError = '';
    try {
      const results = await Promise.all(links.map(l => admin.resolveLink(l)));
      known = results.filter(r => r.existing);
      pages = results.filter(r => !r.existing && r.kind === 'web_page');
      others = results.filter(r => !r.existing && r.kind !== 'web_page');
      resolved = pages[0] ?? others[0] ?? known[0] ?? null;
      const first = pages[0];
      sameParent = !!first?.parent && pages.every(p => p.parent?.id === first.parent?.id);
      if (first) {
        if (sameParent) {
          pageMode = 'parent';
        } else if (first.suggestion) {
          pageMode = 'site';
          siteRoot = first.suggestion.root;
          form.title = first.suggestion.site_name;
        } else {
          pageMode = 'single';
        }
      }
    } catch (e) {
      addError = errorMessage(e);
    } finally {
      resolving = false;
    }
  }

  async function addSource() {
    if (!resolved) return;
    adding = true; addError = '';
    const done: string[] = [];
    try {
      const base: NewSourcePayload = { ...form };
      if (!base.valid_from) delete base.valid_from;
      if (!base.supersedes_id) base.supersedes_id = null;
      const pageUrls = pages.map(p => p.url);
      if (pageUrls.length) {
        if (pageMode === 'parent' && sameParent && pages[0].parent) {
          done.push((await admin.addPages(pages[0].parent.id, pageUrls)).message);
        } else if (pageMode === 'site') {
          done.push((await admin.createSource({ ...base, url: siteRoot.trim(), kind: 'web_site', pages: pageUrls, discover })).message);
        } else {
          for (const url of pageUrls) done.push((await admin.createSource({ ...base, url, kind: 'web_page' })).message);
        }
      }
      // PDFs and Drive links: one source per link, each with its own kind
      const shared = pageUrls.length > 0; // the title typed above belongs to the pages
      for (const r of others) {
        const kind = r.kind === 'web_pdf' ? 'web_pdf' : undefined;
        done.push((await admin.createSource({ ...base, title: shared ? '' : base.title, url: r.url, kind })).message);
      }
      showAdd = false;
      notice = done.length > 1 ? `${done.length} added. ${done[done.length - 1]}` : (done[0] ?? 'Nothing to add.');
      await load();
    } catch (e) {
      addError = errorMessage(e) + (done.length ? ` (${done.length} already added before the error)` : '');
      if (done.length) await load();
    } finally {
      adding = false;
    }
  }

  // ── Sources ──
  async function check(src: SourceRow) {
    try {
      const res = await admin.checkSource(src.id);
      notice = res.message ?? (res.queued ? 'Queued.' : 'Already waiting in the queue.');
      await load();
    } catch (e) { error = errorMessage(e); }
  }

  // ── Edit source details (title, version, valid from) ──
  let editingSrc: SourceRow | null = null;
  let srcForm: SourceUpdatePayload = { title: '', category: '', subcategory: '', version_label: '', valid_from: '', auto_check: true };
  let savingSrc = false;
  let srcError = '';

  function startEditSource(src: SourceRow) {
    editingSrc = src;
    srcForm = {
      title: src.title, category: src.category, subcategory: src.subcategory,
      version_label: src.version_label, valid_from: src.valid_from ?? '', auto_check: src.auto_check
    };
    srcError = '';
  }

  async function saveSource() {
    if (!editingSrc) return;
    savingSrc = true; srcError = '';
    try {
      await admin.updateSource(editingSrc.id, srcForm);
      editingSrc = null;
      notice = 'Source updated. The version applies to its documents right away.';
      await load();
    } catch (e) {
      srcError = errorMessage(e);
    } finally {
      savingSrc = false;
    }
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

  async function toggleFiles(src: SourceRow) {
    if (openFiles === src.id) { openFiles = null; return; }
    openFiles = src.id;
    try { files = { ...files, [src.id]: await admin.sourceFiles(src.id) }; }
    catch (e) { error = errorMessage(e); }
  }

  async function addPagesTo(src: SourceRow) {
    const urls = (newPages[src.id] ?? '').split(/\s+/).map(l => l.trim()).filter(Boolean);
    if (!urls.length) return;
    busy = { ...busy, [`pages-${src.id}`]: true };
    try {
      notice = (await admin.addPages(src.id, urls)).message;
      newPages = { ...newPages, [src.id]: '' };
      await load();
    } catch (e) { error = errorMessage(e); }
    finally { busy = { ...busy, [`pages-${src.id}`]: false }; }
  }

  function relativePath(url: string, root: string): string {
    if (url === root) return '(home page)';
    return url.startsWith(root) ? url.slice(root.length) : url;
  }

  // ── Review ──
  interface DraftGroup { key: string; sourceId: number | null; title: string; drafts: DraftDocument[] }
  $: groups = drafts.reduce<DraftGroup[]>((acc, d) => {
    const key = d.source_id === null ? `doc-${d.document_id}` : `src-${d.source_id}`;
    let g = acc.find(x => x.key === key);
    if (!g) {
      g = { key, sourceId: d.source_id, title: d.source_title || d.title, drafts: [] };
      acc.push(g);
    }
    g.drafts.push(d);
    return acc;
  }, []);

  async function toggleDraft(d: DraftDocument) {
    if (openDraft === d.document_id) { openDraft = null; return; }
    openDraft = d.document_id;
    if (!draftChunks[d.document_id]) {
      try { draftChunks = { ...draftChunks, [d.document_id]: await admin.draftChunks(d.document_id) }; }
      catch (e) { error = errorMessage(e); }
    }
  }

  async function approve(d: DraftDocument) {
    const replaces = d.replaces.length ? `\n\nThis archives (replaces):\n• ${d.replaces.join('\n• ')}` : '';
    if (!confirm(`Approve "${d.title}"? It will be used in answers right away.${replaces}`)) return;
    busy = { ...busy, [d.document_id]: true };
    try {
      const res = await admin.approveDraft(d.document_id);
      notice = `Approved: ${res.activated_chunks} chunks are now searchable` +
        (res.archived_documents.length ? `; ${res.archived_documents.length} older version(s) archived.` : '.');
      await load();
    } catch (e) { error = errorMessage(e); }
    finally { busy = { ...busy, [d.document_id]: false }; }
  }

  async function approveGroup(g: DraftGroup) {
    if (g.sourceId === null) return;
    if (!confirm(`Approve all ${g.drafts.length} drafts of "${g.title}"? They will be used in answers right away; replaced versions are archived.`)) return;
    busy = { ...busy, [g.key]: true };
    try {
      const res = await admin.approveAll(g.sourceId);
      notice = `Approved ${res.approved_documents} documents (${res.activated_chunks} chunks)` +
        (res.archived_documents.length ? `; ${res.archived_documents.length} older version(s) archived.` : '.');
      await load();
    } catch (e) { error = errorMessage(e); }
    finally { busy = { ...busy, [g.key]: false }; }
  }

  async function reject(d: DraftDocument) {
    if (!confirm(`Reject "${d.title}"? The draft is deleted. It is processed again only when it changes or you press "Check now".`)) return;
    busy = { ...busy, [d.document_id]: true };
    try {
      await admin.rejectDraft(d.document_id);
      notice = 'Draft rejected.';
      await load();
    } catch (e) { error = errorMessage(e); }
    finally { busy = { ...busy, [d.document_id]: false }; }
  }

  const WARN = /angka di PDF|tanpa teks|gagal|GAGAL|duplikat|sama dengan|tidak dideskripsikan|hanya \d+ pertama|Very little text|may need JavaScript|tidak ditemukan/;

  /** Warning lines of the job log that concern this draft (web jobs log one line per page). */
  function warnings(d: DraftDocument): string[] {
    const lines = (d.job_log ?? '').split('\n').filter(l => WARN.test(l));
    const tag = (l: string) => (l.startsWith('[') ? l.slice(1, l.indexOf(']')) : '');
    const mine = lines.filter(l => {
      const t = tag(l);
      return t && (d.citation_url.endsWith(t) || (t === 'halaman utama' && d.citation_url.endsWith('/')));
    });
    if (mine.length) return mine;
    // Web jobs tag lines with page paths; PDF jobs (one job per file) tag them with the file name
    const pageTagged = lines.some(l => /^\[(halaman utama|[^\]]*\/[^\]]*|[^\]]*\.html?)\]/.test(l));
    return pageTagged ? [] : lines;
  }
</script>

<svelte:head><title>Sources · Admin</title></svelte:head>

<!-- Add source -->
<Modal open={showAdd} title="Add a source" on:close={() => (showAdd = false)}>
  <form id="add-source" on:submit|preventDefault={() => (resolved ? addSource() : checkLinks())} class="space-y-4">
    {#if !resolved}
      <div>
        <label for="src-links" class="label">Link(s)</label>
        <textarea id="src-links" bind:value={linksText} rows="4" required class="input font-mono text-xs"
          placeholder={'One link per line, for example:\nhttps://mtiugm.github.io/panduan_pmpsti/chapter1/introduction.html\nhttps://drive.google.com/drive/folders/…\nhttps://jteti.ugm.ac.id/…/panduan.pdf'}></textarea>
        <ul class="text-xs text-muted-foreground mt-2 space-y-1 list-disc pl-4">
          <li><strong>Web pages</strong>: add only the pages you want. If they belong to a site, they are grouped under it.</li>
          <li><strong>PDFs</strong>: a link that opens the .pdf file, or a Google Drive file/folder shared as <strong>Anyone with the link can view</strong>.</li>
        </ul>
      </div>
    {:else}
      <button type="button" class="btn-ghost btn-sm -ml-2" on:click={() => (resolved = null)}><ArrowLeft size={14} /> Change links</button>
      <div class="rounded-lg border bg-muted/30 p-3 text-xs space-y-2">
        {#if pages.length}
          <div>
            <p class="font-semibold text-sm">{pages.length} web {pages.length === 1 ? 'page' : 'pages'}</p>
            {#each pages.slice(0, 4) as r}<p class="font-mono truncate text-muted-foreground">{r.url}</p>{/each}
            {#if pages.length > 4}<p class="text-muted-foreground">… and {pages.length - 4} more</p>{/if}
          </div>
        {/if}
        {#if others.length}
          <div>
            <p class="font-semibold text-sm">{others.length} {others.length === 1 ? 'file link' : 'file links'} (one source each)</p>
            {#each others as r}<p class="font-mono truncate text-muted-foreground">{kindInfo[r.kind]?.label ?? r.kind} · {r.url}</p>{/each}
          </div>
        {/if}
        {#if known.length}
          <div>
            <p class="font-semibold text-sm">{known.length} already added (skipped)</p>
            {#each known as r}<p class="truncate text-muted-foreground"><span class="font-mono">{r.url}</span> → part of “{r.existing?.title}”</p>{/each}
          </div>
        {/if}
      </div>

      {#each [...pages, ...others].filter(r => r.warning) as r}<Alert variant="warning">{r.warning}</Alert>{/each}
      {#if pages.length}
        <fieldset class="space-y-2">
          <legend class="label">How should these pages be stored?</legend>
          {#if sameParent && pages[0].parent}
            <label class="flex gap-2 items-start rounded-lg border p-3 cursor-pointer {pageMode === 'parent' ? 'border-primary bg-primary/5' : ''}">
              <input type="radio" bind:group={pageMode} value="parent" class="mt-1" />
              <span class="text-sm">
                <span class="font-semibold">Add to “{pages[0].parent.title || pages[0].parent.root}”</span>
                <span class="block text-xs text-muted-foreground">These pages lie under {pages[0].parent.root}, which is already a source.</span>
              </span>
            </label>
          {:else}
            <label class="flex gap-2 items-start rounded-lg border p-3 cursor-pointer {pageMode === 'site' ? 'border-primary bg-primary/5' : ''}">
              <input type="radio" bind:group={pageMode} value="site" class="mt-1" />
              <span class="text-sm flex-1 min-w-0">
                <span class="font-semibold">Part of a website{pages[0].suggestion?.site_name ? `: ${pages[0].suggestion.site_name}` : ''}</span>
                <span class="block text-xs text-muted-foreground">One source for the whole site; each page stays its own document and citation.</span>
                {#if pageMode === 'site'}
                  <label for="src-root" class="label mt-2">Site address (root)</label>
                  <input id="src-root" bind:value={siteRoot} class="input font-mono text-xs" required />
                  <span class="flex flex-col sm:flex-row gap-2 mt-2 text-xs">
                    <label class="flex items-center gap-1.5"><input type="radio" bind:group={discover} value="manual" /> Only these pages (add more later)</label>
                    <label class="flex items-center gap-1.5"><input type="radio" bind:group={discover} value="auto" /> Also find all pages under the root</label>
                  </span>
                {/if}
              </span>
            </label>
          {/if}
          <label class="flex gap-2 items-start rounded-lg border p-3 cursor-pointer {pageMode === 'single' ? 'border-primary bg-primary/5' : ''}">
            <input type="radio" bind:group={pageMode} value="single" class="mt-1" />
            <span class="text-sm">
              <span class="font-semibold">Standalone {pages.length === 1 ? 'page' : 'pages'}</span>
              <span class="block text-xs text-muted-foreground">Each link is its own source (e.g. a staff page on the department website).</span>
            </span>
          </label>
        </fieldset>
      {/if}

      {#if (pages.length && !(pageMode === 'parent' && sameParent)) || (!pages.length && others.length)}
        <div>
          <label for="src-title" class="label">Title</label>
          <input id="src-title" bind:value={form.title} class="input"
            placeholder={pages.length && pageMode === 'single' ? 'Leave empty to use the page title' : 'e.g. Buku Panduan Akademik PMPSTI'} />
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
        </div>
        <label class="flex items-center gap-2 text-sm">
          <input type="checkbox" bind:checked={form.auto_check} class="rounded border" />
          Check for changes automatically (daily)
        </label>
      {/if}
    {/if}
    {#if addError}<Alert>{addError}</Alert>{/if}
  </form>
  <svelte:fragment slot="footer">
    <button type="button" class="btn-outline" disabled={adding || resolving} on:click={() => (showAdd = false)}>Cancel</button>
    {#if !resolved}
      <button type="submit" form="add-source" class="btn-primary" disabled={resolving || !links.length}>
        {#if resolving}<Loader2 size={15} class="animate-spin" />{:else}<Search size={15} />{/if} Check links
      </button>
    {:else}
      <button type="submit" form="add-source" class="btn-primary" disabled={adding || !(pages.length || others.length)}>
        {#if adding}<Loader2 size={15} class="animate-spin" />{:else}<Plus size={15} />{/if}
        {pages.length && pageMode === 'parent' && sameParent && !others.length ? 'Add pages' : 'Add'}
      </button>
    {/if}
  </svelte:fragment>
</Modal>

<PageHeader title="Sources" description="Where documents come from: web pages, website PDFs and Google Drive. New and changed content waits here for review before it is used in answers.">
  <button on:click={load} class="btn-outline" disabled={loading}>
    <RefreshCw size={15} class={loading ? 'animate-spin' : ''} /> <span class="hidden sm:inline">Refresh</span>
  </button>
  <button on:click={openAdd} class="btn-primary"><Plus size={15} /> Add source</button>
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
      {#each groups as g (g.key)}
        <div class="card overflow-hidden">
          {#if g.sourceId !== null && g.drafts.length > 1}
            <div class="flex items-center justify-between gap-3 px-4 py-3 border-b bg-muted/30">
              <p class="text-sm font-semibold min-w-0 truncate">{g.title} <span class="text-muted-foreground font-normal">· {g.drafts.length} drafts</span></p>
              <button class="btn-primary btn-sm shrink-0" disabled={busy[g.key]} on:click={() => approveGroup(g)}>
                {#if busy[g.key]}<Loader2 size={14} class="animate-spin" />{:else}<Check size={14} />{/if} Approve all
              </button>
            </div>
          {/if}
          <ul class="divide-y">
            {#each g.drafts as d (d.document_id)}
              {@const open = openDraft === d.document_id}
              {@const warn = warnings(d)}
              <li class="p-4 space-y-3">
                <div class="flex flex-col sm:flex-row sm:items-start gap-3">
                  <button on:click={() => toggleDraft(d)} aria-expanded={open} class="flex-1 min-w-0 text-left flex gap-3">
                    <span class="w-9 h-9 rounded-lg bg-primary/10 text-primary flex items-center justify-center shrink-0"><FileText size={17} /></span>
                    <span class="min-w-0 flex-1">
                      <span class="block font-semibold text-sm leading-snug">{g.drafts.length > 1 || !d.source_title ? d.title : d.source_title}</span>
                      <span class="flex flex-wrap items-center gap-1.5 mt-1.5 text-xs">
                        {#if d.version_label}<span class="badge-primary">{d.version_label}</span>{/if}
                        {#if d.page_count > 1}<span class="badge-neutral">{d.page_count} pages</span>{/if}
                        <span class="badge-neutral">{d.chunk_count} chunks</span>
                        {#if d.table_chunks}<span class="badge-neutral"><Table2 size={10} /> {d.table_chunks}</span>{/if}
                        {#if d.figure_chunks}<span class="badge-neutral"><Image size={10} /> {d.figure_chunks}</span>{/if}
                        {#if d.replaces.length}<span class="badge-neutral">replaces {d.replaces.length}</span>{/if}
                        {#if warn.length}<span class="badge-warning"><AlertTriangle size={10} /> {warn.length} warnings</span>{/if}
                      </span>
                      <span class="block text-xs text-muted-foreground mt-1 truncate">{d.citation_url}</span>
                    </span>
                    <ChevronDown size={16} class="shrink-0 mt-2 text-muted-foreground transition-transform {open ? 'rotate-180' : ''}" />
                  </button>
                  <div class="flex items-center gap-2 shrink-0">
                    {#if d.citation_url?.startsWith('http')}
                      <a href={d.citation_url} target="_blank" rel="noopener noreferrer" class="icon-btn" title="Open the original" aria-label="Open the original"><ExternalLink size={15} /></a>
                    {/if}
                    <button class="btn-outline btn-sm" disabled={busy[d.document_id]} on:click={() => reject(d)}><X size={14} /> Reject</button>
                    <button class="btn-primary btn-sm" disabled={busy[d.document_id]} on:click={() => approve(d)}>
                      {#if busy[d.document_id]}<Loader2 size={14} class="animate-spin" />{:else}<Check size={14} />{/if} Approve
                    </button>
                  </div>
                </div>

                {#if open}
                  <div class="space-y-3" transition:slide={{ duration: 150 }}>
                    {#if d.replaces.length}
                      <Alert variant="info">Approving archives: {d.replaces.join(', ')}</Alert>
                    {/if}
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
                              {#if c.page_number}<span class="font-mono text-muted-foreground shrink-0">p.{c.page_number}</span>{/if}
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
        </div>
      {/each}
    {/if}
  </section>

  <!-- Sources -->
  <section aria-labelledby="sources-heading" class="space-y-3">
    <h2 id="sources-heading" class="text-sm font-bold uppercase tracking-[0.12em] text-muted-foreground">Sources</h2>
    {#if !sources.length && !loading}
      <div class="card">
        <EmptyState icon={FolderSync} title="No sources yet"
          description="Add pages of the guidebook website, a PDF link from the department website, or a shared Google Drive folder." />
      </div>
    {:else}
      <ul class="card divide-y overflow-hidden">
        {#each sources as s (s.id)}
          {@const info = kindInfo[s.kind] ?? kindInfo.web_page}
          {@const multi = s.kind === 'web_site' || s.kind === 'drive_folder'}
          <li class="p-4">
            <div class="flex flex-col sm:flex-row sm:items-start gap-3">
              <div class="flex-1 min-w-0 flex gap-3">
                <span class="w-9 h-9 rounded-lg bg-muted text-muted-foreground flex items-center justify-center shrink-0">
                  <svelte:component this={info.icon} size={17} />
                </span>
                <div class="min-w-0">
                  <p class="font-semibold text-sm leading-snug">{s.title || hostname(s.url) || s.url}</p>
                  <p class="text-xs text-muted-foreground truncate">{s.url}</p>
                  <p class="flex flex-wrap items-center gap-1.5 mt-1.5 text-xs">
                    <span class="badge-neutral">{info.label}{s.kind === 'web_site' ? ` · ${s.discover === 'auto' ? 'finds new pages' : 'selected pages'}` : ''}</span>
                    {#if s.version_label || s.valid_from}<span class="badge-primary" title="Version used when sources disagree">{[s.version_label, s.valid_from].filter(Boolean).join(' · ')}</span>{/if}
                    {#if s.category}<span class="badge-neutral">{s.category}{s.subcategory ? ` / ${s.subcategory}` : ''}</span>{/if}
                    <span class="badge-neutral">
                      {#if multi}{s.file_count} {s.kind === 'web_site' ? (s.file_count === 1 ? 'page' : 'pages') : 'files'} · {/if}{s.active_files} active · {s.draft_files} in review
                    </span>
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
              <div class="flex items-center gap-1 shrink-0 flex-wrap">
                <a href={s.url} target="_blank" rel="noopener noreferrer" class="icon-btn" title="Open link" aria-label="Open link"><ExternalLink size={15} /></a>
                <button class="btn-outline btn-sm" on:click={() => check(s)} disabled={s.last_job_state === 'queued' || s.last_job_state === 'running'}>
                  <RefreshCw size={13} /> Check now
                </button>
                {#if multi}
                  <button class="btn-ghost btn-sm" on:click={() => toggleFiles(s)} aria-expanded={openFiles === s.id}>{s.kind === 'web_site' ? 'Pages' : 'Files'}</button>
                {/if}
                <button class="btn-ghost btn-sm" on:click={() => toggleJobs(s)} aria-expanded={openJobs === s.id}>History</button>
                <button class="icon-btn" on:click={() => startEditSource(s)} title="Edit details" aria-label="Edit source"><Pencil size={15} /></button>
                <button class="icon-btn hover:!text-destructive hover:!bg-destructive/10" on:click={() => remove(s)} title="Remove source" aria-label="Remove source"><Trash2 size={15} /></button>
              </div>
            </div>

            {#if openFiles === s.id}
              <div class="mt-3 sm:pl-12 space-y-2" transition:slide={{ duration: 150 }}>
                {#if !files[s.id]}
                  <p class="flex items-center gap-2 text-sm text-muted-foreground"><Loader2 size={14} class="animate-spin" /> Loading…</p>
                {:else}
                  <ul class="rounded-lg border divide-y text-xs">
                    {#each files[s.id] as f (f.id)}
                      <li class="flex items-center gap-2 px-3 py-2 min-w-0">
                        <span class={fileStatus[f.status] ?? 'badge-neutral'}>{f.status}</span>
                        <span class="truncate flex-1" title={f.citation_url}>{f.name || relativePath(f.external_id, s.url)}</span>
                        <a href={f.citation_url} target="_blank" rel="noopener noreferrer" class="text-muted-foreground hover:text-primary shrink-0 font-mono truncate max-w-[45%]">{relativePath(f.external_id, s.url)}</a>
                      </li>
                    {:else}
                      <li class="px-3 py-2 text-muted-foreground">No {s.kind === 'web_site' ? 'pages' : 'files'} yet.</li>
                    {/each}
                  </ul>
                  {#if s.kind === 'web_site'}
                    <div class="flex flex-col sm:flex-row gap-2">
                      <textarea bind:value={newPages[s.id]} rows="2" class="input font-mono text-xs flex-1"
                        placeholder={`More pages under ${s.url}, one per line`}></textarea>
                      <button class="btn-outline btn-sm self-start" disabled={busy[`pages-${s.id}`] || !(newPages[s.id] ?? '').trim()} on:click={() => addPagesTo(s)}>
                        {#if busy[`pages-${s.id}`]}<Loader2 size={13} class="animate-spin" />{:else}<Plus size={13} />{/if} Add pages
                      </button>
                    </div>
                  {/if}
                {/if}
              </div>
            {/if}

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

<!-- Edit source -->
<Modal open={!!editingSrc} title="Edit source" on:close={() => (editingSrc = null)}>
  <form id="edit-source" on:submit|preventDefault={saveSource} class="space-y-4">
    <p class="text-xs text-muted-foreground font-mono truncate">{editingSrc?.url}</p>
    <div>
      <label for="es-title" class="label">Title</label>
      <input id="es-title" bind:value={srcForm.title} class="input" />
    </div>
    <div class="grid sm:grid-cols-2 gap-3">
      <div>
        <label for="es-version" class="label">Version</label>
        <input id="es-version" bind:value={srcForm.version_label} class="input" placeholder="e.g. 2022 Revisi-2" />
      </div>
      <div>
        <label for="es-valid" class="label">Valid from</label>
        <input id="es-valid" type="date" bind:value={srcForm.valid_from} class="input" />
      </div>
      <div>
        <label for="es-cat" class="label">Category</label>
        <input id="es-cat" bind:value={srcForm.category} class="input" />
      </div>
      <div>
        <label for="es-subcat" class="label">Subcategory</label>
        <input id="es-subcat" bind:value={srcForm.subcategory} class="input" />
      </div>
    </div>
    <p class="text-xs text-muted-foreground">
      When sources disagree (for example SKS totals), answers follow the newest “valid from” and mention the older value.
      Version and date apply to this source's documents right away; title and category apply to pages processed from now on.
    </p>
    <label class="flex items-center gap-2 text-sm">
      <input type="checkbox" bind:checked={srcForm.auto_check} class="rounded border" />
      Check for changes automatically (daily)
    </label>
    {#if srcError}<Alert>{srcError}</Alert>{/if}
  </form>
  <svelte:fragment slot="footer">
    <button type="button" class="btn-outline" disabled={savingSrc} on:click={() => (editingSrc = null)}>Cancel</button>
    <button type="submit" form="edit-source" class="btn-primary" disabled={savingSrc}>
      {#if savingSrc}<Loader2 size={15} class="animate-spin" />{/if} Save
    </button>
  </svelte:fragment>
</Modal>

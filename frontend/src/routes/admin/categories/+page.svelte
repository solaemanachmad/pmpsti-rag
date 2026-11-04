<script lang="ts">
  // Topic categories (two levels) used to file documents in Admin → Documents.
  // Add, rename (the slug stays), reorder, and delete — a delete can move the documents
  // to another category first (merge).
  import { onMount } from 'svelte';
  import { admin } from '$lib/api/client';
  import type { Category } from '$lib/api/client';
  import { errorMessage } from '$lib/format';
  import PageHeader from '$lib/components/ui/PageHeader.svelte';
  import Alert from '$lib/components/ui/Alert.svelte';
  import LoadingState from '$lib/components/ui/LoadingState.svelte';
  import Modal from '$lib/components/ui/Modal.svelte';
  import { RefreshCw, Plus, Pencil, Trash2, ArrowUp, ArrowDown, Check, X, Folder } from 'lucide-svelte';

  let categories: Category[] = [];
  let uncategorized = 0;
  let loading = false;
  let busy = false;
  let error = '';
  let notice = '';

  /** id being renamed, and the name typed */
  let editingId: number | null = null;
  let editName = '';
  /** "add" form: '' = top level, otherwise the parent id */
  let newName = '';
  let newParent = '';
  /** delete dialog */
  let deleting: Category | null = null;
  let moveTo = '';

  onMount(load);

  async function load() {
    loading = true; error = '';
    try {
      const res = await admin.categories();
      categories = res.categories ?? [];
      uncategorized = res.uncategorized ?? 0;
    } catch (e) { error = errorMessage(e); }
    finally { loading = false; }
  }

  async function run(fn: () => Promise<unknown>, done: string) {
    busy = true; error = ''; notice = '';
    try { await fn(); notice = done; await load(); return true; }
    catch (e) { error = errorMessage(e); return false; }
    finally { busy = false; }
  }

  $: parents = categories.filter(c => c.parent_id == null);
  $: childrenOf = (id: number) => categories.filter(c => c.parent_id === id);
  $: byId = new Map(categories.map(c => [c.id, c]));
  const pathOf = (c: Category) => {
    const p = c.parent_id != null ? byId.get(c.parent_id) : undefined;
    return p ? `${p.name} › ${c.name}` : c.name;
  };
  /** documents in a category and its subcategories */
  const totalOf = (c: Category) => c.document_count + childrenOf(c.id).reduce((n, k) => n + k.document_count, 0);

  async function add() {
    const name = newName.trim();
    if (!name) return;
    const parent = newParent ? Number(newParent) : null;
    const where = parent != null ? ` under ${byId.get(parent)?.name}` : '';
    if (await run(() => admin.createCategory(name, parent), `“${name}” added${where}.`)) newName = '';
  }

  function startRename(c: Category) { editingId = c.id; editName = c.name; }
  async function saveRename(c: Category) {
    const name = editName.trim();
    if (!name || name === c.name) { editingId = null; return; }
    if (await run(() => admin.renameCategory(c.id, name), `“${c.name}” renamed to “${name}”.`)) editingId = null;
  }

  /** move a category one place up (-1) or down (+1) among its siblings */
  function move(c: Category, step: -1 | 1) {
    const siblings = c.parent_id == null ? parents : childrenOf(c.parent_id);
    const ids = siblings.map(s => s.id);
    const i = ids.indexOf(c.id), j = i + step;
    if (j < 0 || j >= ids.length) return;
    [ids[i], ids[j]] = [ids[j], ids[i]];
    run(() => admin.reorderCategories(c.parent_id, ids), `“${c.name}” moved ${step < 0 ? 'up' : 'down'}.`);
  }

  function askDelete(c: Category) { deleting = c; moveTo = ''; }
  async function confirmDelete() {
    if (!deleting) return;
    const c = deleting;
    const target = moveTo ? Number(moveTo) : null;
    const where = target != null ? `moved to ${pathOf(byId.get(target)!)}` : 'now uncategorized';
    const ok = await run(() => admin.deleteCategory(c.id, target),
      `“${c.name}” deleted${c.document_count ? `; its ${c.document_count} document(s) are ${where}` : ''}.`);
    if (ok) deleting = null;
  }
  /** where a deleted category's documents can go: any other category */
  $: moveOptions = deleting ? categories.filter(c => c.id !== deleting?.id) : [];
</script>

<svelte:head><title>Categories · Admin</title></svelte:head>

<PageHeader title="Categories" description="Topics used to file documents. Renaming keeps links and rules working.">
  <button on:click={load} class="btn-outline" disabled={loading}>
    <RefreshCw size={15} class={loading ? 'animate-spin' : ''} /> <span class="hidden sm:inline">Refresh</span>
  </button>
</PageHeader>

<div class="space-y-6">
  {#if notice}<Alert variant="success" dismissible on:dismiss={() => (notice = '')}>{notice}</Alert>{/if}
  {#if error}<Alert dismissible on:dismiss={() => (error = '')}>{error}</Alert>{/if}

  <!-- Add -->
  <section class="card px-5 py-4">
    <h2 class="font-bold mb-3">Add a category</h2>
    <form class="flex flex-col sm:flex-row gap-2" on:submit|preventDefault={add}>
      <input bind:value={newName} placeholder="Name, e.g. Beasiswa" class="input flex-1" aria-label="New category name" maxlength="80" />
      <select bind:value={newParent} class="input sm:w-64" aria-label="Parent">
        <option value="">Top level</option>
        {#each parents as p (p.id)}<option value={String(p.id)}>Under {p.name}</option>{/each}
      </select>
      <button type="submit" class="btn-primary shrink-0" disabled={busy || !newName.trim()}><Plus size={15} /> Add</button>
    </form>
  </section>

  <!-- Tree -->
  <section class="card overflow-hidden">
    <div class="px-5 py-4 border-b">
      <h2 class="font-bold">All categories <span class="text-sm font-normal text-muted-foreground">· {categories.length}</span></h2>
      <p class="text-xs text-muted-foreground">
        Document counts exclude archived documents.
        {#if uncategorized}<a href="/admin/documents" class="text-primary hover:underline">{uncategorized} uncategorized</a>.{/if}
      </p>
    </div>
    {#if loading && !categories.length}
      <LoadingState />
    {:else}
      <ul class="divide-y">
        {#each parents as p, i (p.id)}
          {@const kids = childrenOf(p.id)}
          <li>
            <div class="flex items-center gap-2 px-5 py-3">
              {#if editingId === p.id}
                <form class="flex-1 flex gap-2" on:submit|preventDefault={() => saveRename(p)}>
                  <input bind:value={editName} class="input flex-1" aria-label="New name for {p.name}" maxlength="80" />
                  <button type="submit" class="btn-primary btn-sm" disabled={busy}><Check size={13} /> Save</button>
                  <button type="button" class="btn-ghost btn-sm" on:click={() => (editingId = null)}><X size={13} /></button>
                </form>
              {:else}
                <Folder size={15} class="shrink-0 text-muted-foreground" />
                <span class="flex-1 min-w-0 truncate font-semibold">{p.name}</span>
                <span class="badge-neutral" title="Documents here and in its subcategories">{totalOf(p)}</span>
                <div class="flex items-center gap-0.5 shrink-0">
                  <button class="icon-btn" title="Move up" aria-label="Move {p.name} up" disabled={busy || i === 0} on:click={() => move(p, -1)}><ArrowUp size={15} /></button>
                  <button class="icon-btn" title="Move down" aria-label="Move {p.name} down" disabled={busy || i === parents.length - 1} on:click={() => move(p, 1)}><ArrowDown size={15} /></button>
                  <button class="icon-btn" title="Rename" aria-label="Rename {p.name}" on:click={() => startRename(p)}><Pencil size={15} /></button>
                  <button class="icon-btn" title={kids.length ? 'Delete its subcategories first' : 'Delete'} aria-label="Delete {p.name}"
                    disabled={busy || kids.length > 0} on:click={() => askDelete(p)}><Trash2 size={15} /></button>
                </div>
              {/if}
            </div>
            {#if kids.length}
              <ul class="ml-6 sm:ml-9 border-l mb-2">
                {#each kids as c, j (c.id)}
                  <li class="flex items-center gap-2 px-5 py-2">
                    {#if editingId === c.id}
                      <form class="flex-1 flex gap-2" on:submit|preventDefault={() => saveRename(c)}>
                        <input bind:value={editName} class="input flex-1" aria-label="New name for {c.name}" maxlength="80" />
                        <button type="submit" class="btn-primary btn-sm" disabled={busy}><Check size={13} /> Save</button>
                        <button type="button" class="btn-ghost btn-sm" on:click={() => (editingId = null)}><X size={13} /></button>
                      </form>
                    {:else}
                      <span class="flex-1 min-w-0 truncate">{c.name}</span>
                      <span class="badge-neutral">{c.document_count}</span>
                      <div class="flex items-center gap-0.5 shrink-0">
                        <button class="icon-btn" title="Move up" aria-label="Move {c.name} up" disabled={busy || j === 0} on:click={() => move(c, -1)}><ArrowUp size={15} /></button>
                        <button class="icon-btn" title="Move down" aria-label="Move {c.name} down" disabled={busy || j === kids.length - 1} on:click={() => move(c, 1)}><ArrowDown size={15} /></button>
                        <button class="icon-btn" title="Rename" aria-label="Rename {c.name}" on:click={() => startRename(c)}><Pencil size={15} /></button>
                        <button class="icon-btn" title="Delete" aria-label="Delete {c.name}" disabled={busy} on:click={() => askDelete(c)}><Trash2 size={15} /></button>
                      </div>
                    {/if}
                  </li>
                {/each}
              </ul>
            {/if}
          </li>
        {/each}
      </ul>
    {/if}
  </section>
</div>

<Modal open={deleting != null} title="Delete “{deleting?.name ?? ''}”" on:close={() => (deleting = null)}>
  {#if deleting}
    {#if deleting.document_count}
      <p class="text-sm mb-3">
        {deleting.document_count} document(s) are filed here. Move them to another category (merge), or leave them uncategorized.
      </p>
      <label class="block text-sm font-medium mb-1" for="move-to">Move documents to</label>
      <select id="move-to" bind:value={moveTo} class="input w-full mb-4">
        <option value="">Leave uncategorized</option>
        {#each moveOptions as o (o.id)}<option value={String(o.id)}>{pathOf(o)}</option>{/each}
      </select>
    {:else}
      <p class="text-sm mb-4">No documents are filed here.</p>
    {/if}
    <div class="flex justify-end gap-2">
      <button type="button" class="btn-ghost" on:click={() => (deleting = null)}>Cancel</button>
      <button type="button" class="btn-danger" disabled={busy} on:click={confirmDelete}><Trash2 size={15} /> Delete</button>
    </div>
  {/if}
</Modal>


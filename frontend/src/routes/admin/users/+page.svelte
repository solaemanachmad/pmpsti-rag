<script lang="ts">
  import { onMount } from 'svelte';
  import { admin } from '$lib/api/client';
  import type { AdminUser } from '$lib/api/client';
  import { currentUser } from '$lib/stores/auth';
  import { errorMessage, formatDate } from '$lib/format';
  import PageHeader from '$lib/components/ui/PageHeader.svelte';
  import Alert from '$lib/components/ui/Alert.svelte';
  import Modal from '$lib/components/ui/Modal.svelte';
  import LoadingState from '$lib/components/ui/LoadingState.svelte';
  import EmptyState from '$lib/components/ui/EmptyState.svelte';
  import { Loader2, Trash2, ShieldCheck, ShieldOff, RefreshCw, Search, UserPlus, MailCheck, Users } from 'lucide-svelte';

  let users: AdminUser[] = [];
  let loading = false;
  let error = '';
  let search = '';
  type Filter = 'all' | 'active' | 'inactive' | 'unverified' | 'admin';
  let filter: Filter = 'all';

  let showAdd = false;
  let addMode: 'single' | 'batch' = 'single';
  let newEmail = ''; let newName = ''; let newRole = 'user';
  let batchCsv = '';
  let addLoading = false; let addError = ''; let addSuccess = '';

  onMount(loadUsers);

  async function loadUsers() {
    loading = true; error = '';
    try { users = await admin.users(); }
    catch (e) { error = errorMessage(e); }
    finally { loading = false; }
  }

  function openAdd() {
    addError = ''; addSuccess = '';
    showAdd = true;
  }

  async function addSingle() {
    if (!newEmail.trim()) { addError = 'Email is required.'; return; }
    addLoading = true; addError = ''; addSuccess = '';
    try {
      await admin.createUser(newEmail.trim(), newName.trim() || undefined, newRole);
      addSuccess = `${newEmail.trim()} was added.`;
      newEmail = ''; newName = ''; newRole = 'user';
      loadUsers();
    } catch (e) { addError = errorMessage(e); }
    finally { addLoading = false; }
  }

  async function addBatch() {
    const lines = batchCsv.trim().split('\n').filter(l => l.trim() && !l.toLowerCase().startsWith('email'));
    if (!lines.length) { addError = 'The CSV is empty.'; return; }
    addLoading = true; addError = ''; addSuccess = '';
    let ok = 0; let fail = 0;
    for (const line of lines) {
      const [email, name, role] = line.split(',').map(s => s.trim());
      if (!email) { fail++; continue; }
      try { await admin.createUser(email, name || undefined, role || 'user'); ok++; }
      catch { fail++; }
    }
    addSuccess = `Imported ${ok} ${ok === 1 ? 'user' : 'users'}${fail ? `, ${fail} failed` : ''}.`;
    addLoading = false;
    if (ok > 0) { batchCsv = ''; loadUsers(); }
  }

  async function act(fn: () => Promise<unknown>, update: (list: AdminUser[]) => AdminUser[]) {
    error = '';
    try { await fn(); users = update(users); }
    catch (e) { error = errorMessage(e); }
  }

  const toggleUser = (u: AdminUser) =>
    act(() => admin.toggleUser(u.id, !u.is_active), l => l.map(x => x.id === u.id ? { ...x, is_active: !u.is_active } : x));

  const setRole = (u: AdminUser, role: string) =>
    act(() => admin.setRole(u.id, role), l => l.map(x => x.id === u.id ? { ...x, role } : x));

  const verifyUser = (u: AdminUser) =>
    act(() => admin.verifyUser(u.id), l => l.map(x => x.id === u.id ? { ...x, email_verified: true, is_active: true } : x));

  function deleteUser(u: AdminUser) {
    if (!confirm(`Delete ${u.email}? This cannot be undone.`)) return;
    act(() => admin.deleteUser(u.id), l => l.filter(x => x.id !== u.id));
  }

  $: counts = {
    all: users.length,
    active: users.filter(u => u.is_active).length,
    inactive: users.filter(u => !u.is_active).length,
    unverified: users.filter(u => !u.email_verified).length,
    admin: users.filter(u => u.role === 'admin').length
  };

  const filters: { key: Filter; label: string }[] = [
    { key: 'all', label: 'All' },
    { key: 'active', label: 'Active' },
    { key: 'inactive', label: 'Inactive' },
    { key: 'unverified', label: 'Unverified' },
    { key: 'admin', label: 'Admins' }
  ];

  $: q = search.trim().toLowerCase();
  $: filtered = users.filter(u => {
    if (filter === 'active' && !u.is_active) return false;
    if (filter === 'inactive' && u.is_active) return false;
    if (filter === 'unverified' && u.email_verified) return false;
    if (filter === 'admin' && u.role !== 'admin') return false;
    return !q || u.email.toLowerCase().includes(q) || (u.display_name ?? '').toLowerCase().includes(q);
  });

  const initials = (u: AdminUser) =>
    (u.display_name || u.email).split(/[\s@.]+/).filter(Boolean).slice(0, 2).map(w => w[0]?.toUpperCase()).join('');
</script>

<svelte:head><title>Users · Admin</title></svelte:head>

<Modal open={showAdd} title="Add users" on:close={() => (showAdd = false)}>
  <div class="flex gap-1 mb-4 p-1 bg-muted rounded-lg" role="tablist">
    {#each [['single', 'Single user'], ['batch', 'Import CSV']] as [mode, label]}
      <button role="tab" aria-selected={addMode === mode} on:click={() => { addMode = mode === 'single' ? 'single' : 'batch'; addError = ''; addSuccess = ''; }}
        class="flex-1 h-8 text-xs rounded-md font-semibold transition-colors
               {addMode === mode ? 'bg-card shadow-sm text-foreground' : 'text-muted-foreground hover:text-foreground'}">
        {label}
      </button>
    {/each}
  </div>

  <form id="add-user" on:submit|preventDefault={addMode === 'single' ? addSingle : addBatch} class="space-y-4">
    {#if addMode === 'single'}
      <div>
        <label for="new-email" class="label">Email</label>
        <input id="new-email" bind:value={newEmail} type="email" placeholder="name@mail.ugm.ac.id" class="input" required />
      </div>
      <div>
        <label for="new-name" class="label">Name <span class="font-normal opacity-70">(optional)</span></label>
        <input id="new-name" bind:value={newName} placeholder="Full name" class="input" />
      </div>
      <div>
        <label for="new-role" class="label">Role</label>
        <select id="new-role" bind:value={newRole} class="input">
          <option value="user">User</option>
          <option value="admin">Admin</option>
        </select>
      </div>
    {:else}
      <div>
        <label for="batch" class="label">One user per line: <code class="font-mono bg-muted px-1 rounded">email,name,role</code></label>
        <textarea id="batch" bind:value={batchCsv} rows={6}
          placeholder={'budi@mail.ugm.ac.id,Budi,user\nsiti@mail.ugm.ac.id,Siti,admin'}
          class="input font-mono text-xs resize-y"></textarea>
      </div>
    {/if}
    {#if addError}<Alert>{addError}</Alert>{/if}
    {#if addSuccess}<Alert variant="success">{addSuccess}</Alert>{/if}
  </form>

  <svelte:fragment slot="footer">
    <button type="button" class="btn-outline" on:click={() => (showAdd = false)}>Close</button>
    <button type="submit" form="add-user" class="btn-primary" disabled={addLoading}>
      {#if addLoading}<Loader2 size={15} class="animate-spin" />{/if}
      {addMode === 'single' ? 'Add user' : 'Import'}
    </button>
  </svelte:fragment>
</Modal>

<PageHeader title="Users" description="{users.length} registered {users.length === 1 ? 'account' : 'accounts'}">
  <button on:click={loadUsers} class="btn-outline" disabled={loading}>
    <RefreshCw size={15} class={loading ? 'animate-spin' : ''} /> <span class="hidden sm:inline">Refresh</span>
  </button>
  <button on:click={openAdd} class="btn-primary"><UserPlus size={15} /> Add users</button>
</PageHeader>

<div class="flex flex-col md:flex-row md:items-center gap-3 mb-4">
  <div class="relative flex-1">
    <Search size={15} class="absolute left-3 top-1/2 -translate-y-1/2 text-muted-foreground" />
    <input bind:value={search} placeholder="Search by email or name…" class="input pl-9" aria-label="Search users" />
  </div>
  <div class="flex gap-1 overflow-x-auto [scrollbar-width:none] -mx-4 px-4 md:mx-0 md:px-0">
    {#each filters as f}
      <button on:click={() => (filter = f.key)}
        class="h-8 px-3 rounded-full text-xs font-semibold whitespace-nowrap border transition-colors
               {filter === f.key ? 'bg-foreground text-background border-foreground' : 'bg-card text-muted-foreground hover:text-foreground'}">
        {f.label} <span class="opacity-60 ml-0.5">{counts[f.key]}</span>
      </button>
    {/each}
  </div>
</div>

{#if error}<div class="mb-4"><Alert dismissible on:dismiss={() => (error = '')}>{error}</Alert></div>{/if}

{#if loading && users.length === 0}
  <LoadingState />
{:else if filtered.length === 0}
  <div class="card"><EmptyState icon={Users} title="No users found" description={q || filter !== 'all' ? 'Try a different search or filter.' : 'Add the first user to get started.'} /></div>
{:else}
  <div class="card overflow-hidden">
    <!-- Desktop -->
    <table class="data-table hidden md:table">
      <thead>
        <tr><th>User</th><th>Role</th><th>Status</th><th>Joined</th><th class="text-right">Actions</th></tr>
      </thead>
      <tbody>
        {#each filtered as u (u.id)}
          {@const isSelf = u.id === $currentUser?.id}
          <tr>
            <td>
              <div class="flex items-center gap-3 min-w-0">
                <span class="w-8 h-8 rounded-full bg-muted text-muted-foreground text-[11px] font-bold flex items-center justify-center shrink-0">{initials(u)}</span>
                <div class="min-w-0">
                  <div class="font-semibold truncate">{u.display_name || '—'} {#if isSelf}<span class="text-xs font-normal text-muted-foreground">(you)</span>{/if}</div>
                  <div class="text-xs text-muted-foreground truncate">{u.email}</div>
                </div>
              </div>
            </td>
            <td>
              <select value={u.role} disabled={isSelf} on:change={e => setRole(u, e.currentTarget.value)}
                class="input h-8 w-28 text-xs {u.role === 'admin' ? 'font-semibold text-primary' : ''}" aria-label="Role for {u.email}">
                <option value="user">User</option>
                <option value="admin">Admin</option>
              </select>
            </td>
            <td>
              <div class="flex flex-wrap gap-1">
                <span class={u.is_active ? 'badge-success' : 'badge-danger'}>{u.is_active ? 'Active' : 'Inactive'}</span>
                {#if !u.email_verified}<span class="badge-warning">Unverified</span>{/if}
              </div>
            </td>
            <td class="text-xs text-muted-foreground whitespace-nowrap">{formatDate(u.created_at)}</td>
            <td>
              <div class="flex justify-end gap-0.5">
                {#if !u.email_verified}
                  <button on:click={() => verifyUser(u)} class="icon-btn hover:!text-warning" title="Verify & activate" aria-label="Verify and activate"><MailCheck size={16} /></button>
                {/if}
                <button on:click={() => toggleUser(u)} disabled={isSelf} class="icon-btn" title={u.is_active ? 'Deactivate' : 'Activate'} aria-label={u.is_active ? 'Deactivate' : 'Activate'}>
                  {#if u.is_active}<ShieldOff size={16} />{:else}<ShieldCheck size={16} />{/if}
                </button>
                <button on:click={() => deleteUser(u)} disabled={isSelf} class="icon-btn hover:!text-destructive hover:!bg-destructive/10" title="Delete" aria-label="Delete user"><Trash2 size={16} /></button>
              </div>
            </td>
          </tr>
        {/each}
      </tbody>
    </table>

    <!-- Mobile -->
    <ul class="md:hidden divide-y">
      {#each filtered as u (u.id)}
        {@const isSelf = u.id === $currentUser?.id}
        <li class="p-4">
          <div class="flex items-start gap-3">
            <span class="w-9 h-9 rounded-full bg-muted text-muted-foreground text-xs font-bold flex items-center justify-center shrink-0">{initials(u)}</span>
            <div class="flex-1 min-w-0">
              <div class="font-semibold text-sm truncate">{u.display_name || '—'} {#if isSelf}<span class="text-xs font-normal text-muted-foreground">(you)</span>{/if}</div>
              <div class="text-xs text-muted-foreground truncate">{u.email}</div>
              <div class="flex flex-wrap gap-1 mt-2">
                <span class={u.is_active ? 'badge-success' : 'badge-danger'}>{u.is_active ? 'Active' : 'Inactive'}</span>
                {#if !u.email_verified}<span class="badge-warning">Unverified</span>{/if}
                <span class="badge-neutral">Joined {formatDate(u.created_at)}</span>
              </div>
            </div>
          </div>
          <div class="flex items-center gap-2 mt-3 pl-12">
            <select value={u.role} disabled={isSelf} on:change={e => setRole(u, e.currentTarget.value)} class="input h-9 text-xs flex-1" aria-label="Role for {u.email}">
              <option value="user">User</option>
              <option value="admin">Admin</option>
            </select>
            {#if !u.email_verified}
              <button on:click={() => verifyUser(u)} class="icon-btn border text-warning" aria-label="Verify and activate"><MailCheck size={16} /></button>
            {/if}
            <button on:click={() => toggleUser(u)} disabled={isSelf} class="icon-btn border" aria-label={u.is_active ? 'Deactivate' : 'Activate'}>
              {#if u.is_active}<ShieldOff size={16} />{:else}<ShieldCheck size={16} />{/if}
            </button>
            <button on:click={() => deleteUser(u)} disabled={isSelf} class="icon-btn border text-destructive" aria-label="Delete user"><Trash2 size={16} /></button>
          </div>
        </li>
      {/each}
    </ul>
  </div>
  <p class="text-xs text-muted-foreground mt-3">Showing {filtered.length} of {users.length}</p>
{/if}

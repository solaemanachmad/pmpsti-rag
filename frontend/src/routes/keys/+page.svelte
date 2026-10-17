<script lang="ts">
  import { keys } from '$lib/api/client';
  import type { ApiKeyInfo, CreateApiKeyResponse } from '$lib/api/client';
  import { authReady } from '$lib/stores/auth';
  import { errorMessage, formatDate } from '$lib/format';
  import PageHeader from '$lib/components/ui/PageHeader.svelte';
  import Alert from '$lib/components/ui/Alert.svelte';
  import Modal from '$lib/components/ui/Modal.svelte';
  import LoadingState from '$lib/components/ui/LoadingState.svelte';
  import EmptyState from '$lib/components/ui/EmptyState.svelte';
  import { Plus, Trash2, Copy, Check, KeyRound, Loader2, X } from 'lucide-svelte';

  let apiKeys: ApiKeyInfo[] = [];
  let loading = false;
  let creating = false;
  let error = '';

  let showForm = false;
  let newName = '';
  let newRateLimit = 30;
  let newExpiresAt = '';

  let newlyCreatedKey: CreateApiKeyResponse | null = null;
  let copied = false;

  let loaded = false;
  $: if ($authReady && !loaded) {
    loaded = true;
    loadKeys();
  }

  async function loadKeys() {
    loading = true;
    try { apiKeys = await keys.list(); }
    catch { error = 'Could not load API keys.'; }
    finally { loading = false; }
  }

  async function createKey() {
    if (!newName.trim()) return;
    creating = true;
    error = '';
    try {
      newlyCreatedKey = await keys.create(newName.trim(), ['search', 'read'], newRateLimit, newExpiresAt || undefined);
      showForm = false;
      newName = '';
      newExpiresAt = '';
      await loadKeys();
    } catch (e) {
      error = errorMessage(e) || 'Could not create the API key.';
    } finally {
      creating = false;
    }
  }

  async function revokeKey(id: number, name: string) {
    if (!confirm(`Revoke the key "${name}"? Apps using it will stop working.`)) return;
    try {
      await keys.revoke(id);
      await loadKeys();
    } catch (e) {
      error = errorMessage(e) || 'Could not revoke the key.';
    }
  }

  async function copyKey(text: string) {
    await navigator.clipboard.writeText(text);
    copied = true;
    setTimeout(() => (copied = false), 2000);
  }
</script>

<svelte:head><title>API Keys · DTETI Answers</title></svelte:head>

<Modal open={showForm} title="Create API key" on:close={() => (showForm = false)}>
  <form id="key-form" on:submit|preventDefault={createKey} class="space-y-4">
    <div>
      <label for="key-name" class="label">Name</label>
      <input id="key-name" bind:value={newName} placeholder="e.g. Production app" class="input" required />
    </div>
    <div class="grid grid-cols-2 gap-3">
      <div>
        <label for="key-ratelimit" class="label">Rate limit (req/min)</label>
        <input id="key-ratelimit" bind:value={newRateLimit} type="number" min="1" max="1000" class="input" />
      </div>
      <div>
        <label for="key-expires" class="label">Expires <span class="font-normal opacity-70">(optional)</span></label>
        <input id="key-expires" bind:value={newExpiresAt} type="date" class="input" />
      </div>
    </div>
  </form>
  <svelte:fragment slot="footer">
    <button type="button" class="btn-outline" on:click={() => (showForm = false)}>Cancel</button>
    <button type="submit" form="key-form" class="btn-primary" disabled={creating || !newName.trim()}>
      {#if creating}<Loader2 size={15} class="animate-spin" />{/if}
      Create key
    </button>
  </svelte:fragment>
</Modal>

<div class="h-full overflow-y-auto">
  <div class="max-w-4xl mx-auto px-4 sm:px-6 py-6 sm:py-10">
    <PageHeader title="API Keys" description="Give your own apps programmatic access to the search API.">
      <button on:click={() => { showForm = true; newlyCreatedKey = null; }} class="btn-primary">
        <Plus size={16} /> New key
      </button>
    </PageHeader>

    <div class="space-y-4">
      {#if error}<Alert dismissible on:dismiss={() => (error = '')}>{error}</Alert>{/if}

      {#if newlyCreatedKey}
        <div class="card border-success/40 p-4">
          <div class="flex items-start gap-3">
            <div class="w-9 h-9 rounded-lg bg-success/10 text-success flex items-center justify-center shrink-0"><KeyRound size={17} /></div>
            <div class="flex-1 min-w-0">
              <p class="text-sm font-bold">Key created — copy it now</p>
              <p class="text-xs text-muted-foreground">For security, it won’t be shown again.</p>
              <div class="flex items-center gap-2 mt-3 rounded-lg border bg-muted/50 pl-3 pr-1 py-1">
                <code class="flex-1 text-xs font-mono truncate">{newlyCreatedKey.key}</code>
                <button on:click={() => newlyCreatedKey && copyKey(newlyCreatedKey.key)} class="btn-sm btn-outline shrink-0">
                  {#if copied}<Check size={13} /> Copied{:else}<Copy size={13} /> Copy{/if}
                </button>
              </div>
            </div>
            <button on:click={() => (newlyCreatedKey = null)} class="icon-btn -mt-1 -mr-1" aria-label="Dismiss"><X size={16} /></button>
          </div>
        </div>
      {/if}

      {#if loading}
        <LoadingState />
      {:else if apiKeys.length === 0}
        <div class="card"><EmptyState icon={KeyRound} title="No API keys yet" description="Create a key to call the API from your own scripts or apps." /></div>
      {:else}
        <div class="card overflow-hidden">
          <!-- Desktop table -->
          <table class="data-table hidden md:table">
            <thead>
              <tr><th>Name</th><th>Prefix</th><th>Created</th><th>Last used</th><th>Status</th><th></th></tr>
            </thead>
            <tbody>
              {#each apiKeys as k (k.id)}
                <tr>
                  <td class="font-semibold">{k.name}</td>
                  <td><code class="text-xs bg-muted px-1.5 py-0.5 rounded font-mono">{k.key_prefix}…</code></td>
                  <td class="text-xs text-muted-foreground">{formatDate(k.created_at)}</td>
                  <td class="text-xs text-muted-foreground">{formatDate(k.last_used_at)}</td>
                  <td><span class={k.is_active ? 'badge-success' : 'badge-neutral'}>{k.is_active ? 'Active' : 'Revoked'}</span></td>
                  <td class="text-right">
                    {#if k.is_active}
                      <button on:click={() => revokeKey(k.id, k.name)} class="icon-btn hover:!text-destructive hover:!bg-destructive/10" title="Revoke key" aria-label="Revoke key">
                        <Trash2 size={15} />
                      </button>
                    {/if}
                  </td>
                </tr>
              {/each}
            </tbody>
          </table>
          <!-- Mobile list -->
          <ul class="md:hidden divide-y">
            {#each apiKeys as k (k.id)}
              <li class="flex items-start gap-3 p-4">
                <div class="flex-1 min-w-0">
                  <div class="flex items-center gap-2">
                    <span class="font-semibold text-sm truncate">{k.name}</span>
                    <span class={k.is_active ? 'badge-success' : 'badge-neutral'}>{k.is_active ? 'Active' : 'Revoked'}</span>
                  </div>
                  <code class="text-xs text-muted-foreground font-mono">{k.key_prefix}…</code>
                  <p class="text-xs text-muted-foreground mt-1">Created {formatDate(k.created_at)} · Last used {formatDate(k.last_used_at)}</p>
                </div>
                {#if k.is_active}
                  <button on:click={() => revokeKey(k.id, k.name)} class="icon-btn text-destructive" aria-label="Revoke key"><Trash2 size={16} /></button>
                {/if}
              </li>
            {/each}
          </ul>
        </div>
      {/if}
    </div>
  </div>
</div>

<script lang="ts">
  import { createEventDispatcher, tick } from 'svelte';
  import { page } from '$app/stores';
  import { goto } from '$app/navigation';
  import {
    Plus, Pencil, Trash2, Check, X, LogOut, UserRound, Moon, Sun, ChevronsUpDown, ShieldCheck, PanelLeftClose
  } from 'lucide-svelte';
  import Logo from './Logo.svelte';
  import { currentUser } from '$lib/stores/auth';
  import { sessions, activeSessionId, renameSession, removeSession } from '$lib/stores/chat';
  import { isDark } from '$lib/theme';
  import { mainNav, adminNav, isNavActive as isActive } from '$lib/nav';

  export let showClose = false;
  /** desktop: show a button that hides the sidebar */
  export let showCollapse = false;

  const dispatch = createEventDispatcher<{ navigate: void; close: void; logout: void; collapse: void }>();

  $: path = $page.url.pathname;
  $: isAdmin = $currentUser?.role === 'admin';
  $: initials = ($currentUser?.display_name || $currentUser?.email || '?')
    .split(/[\s@.]+/).filter(Boolean).slice(0, 2).map(w => w[0]?.toUpperCase()).join('');

  let accountOpen = false;
  let renamingId: string | null = null;
  let renameValue = '';
  let renameInput: HTMLInputElement;

  function navigated() {
    accountOpen = false;
    dispatch('navigate');
  }

  async function newChat() {
    navigated();
    await goto('/chat');
  }

  async function startRename(id: string, title: string) {
    renamingId = id;
    renameValue = title;
    await tick();
    renameInput?.select();
  }

  async function saveRename() {
    const id = renamingId;
    const title = renameValue.trim();
    renamingId = null;
    if (id && title) await renameSession(id, title).catch(() => {});
  }

  async function deleteChat(id: string, title: string) {
    if (!confirm(`Delete "${title || 'Untitled chat'}"? This cannot be undone.`)) return;
    await removeSession(id).catch(() => {});
    if ($activeSessionId === id) goto('/chat');
  }

  const linkBase = 'group relative flex items-center gap-3 h-9 px-3 rounded-lg text-sm font-medium transition-colors';
  const linkIdle = 'text-sidebar-muted hover:bg-sidebar-hover hover:text-sidebar-foreground';
  const linkActive = 'bg-sidebar-hover text-sidebar-foreground before:absolute before:left-0 before:top-2 before:bottom-2 before:w-[3px] before:rounded-r-full before:bg-accent';
</script>

<div class="flex flex-col h-full bg-sidebar text-sidebar-foreground">
  <!-- Brand -->
  <div class="flex items-center justify-between gap-2 h-16 px-4 shrink-0">
    <a href="/chat" on:click={navigated} class="min-w-0"><Logo onDark /></a>
    {#if showClose}
      <button class="w-9 h-9 rounded-lg flex items-center justify-center text-sidebar-muted hover:bg-sidebar-hover hover:text-sidebar-foreground"
        on:click={() => dispatch('close')} aria-label="Close menu">
        <X size={18} />
      </button>
    {:else if showCollapse}
      <button class="w-9 h-9 rounded-lg flex items-center justify-center text-sidebar-muted hover:bg-sidebar-hover hover:text-sidebar-foreground"
        on:click={() => dispatch('collapse')} aria-label="Hide sidebar" title="Hide sidebar">
        <PanelLeftClose size={18} />
      </button>
    {/if}
  </div>

  <div class="px-3 pb-2 shrink-0">
    <button on:click={newChat}
      class="w-full h-10 flex items-center justify-center gap-2 rounded-lg bg-accent text-accent-foreground
             text-sm font-bold hover:brightness-105 transition shadow-sm">
      <Plus size={16} strokeWidth={2.5} /> New chat
    </button>
  </div>

  <div class="flex-1 min-h-0 overflow-y-auto px-3 pb-3 space-y-5">
    <nav class="space-y-0.5 pt-2" aria-label="Main">
      {#each mainNav as item}
        <a href={item.href} on:click={navigated} class="{linkBase} {isActive(item.href, path) ? linkActive : linkIdle}"
           aria-current={isActive(item.href, path) ? 'page' : undefined}>
          <svelte:component this={item.icon} size={17} />
          {item.label}
        </a>
      {/each}
    </nav>

    {#if isAdmin}
      <nav aria-label="Admin">
        <p class="flex items-center gap-1.5 px-3 mb-1 text-[10px] font-bold uppercase tracking-[0.12em] text-sidebar-muted/80">
          <ShieldCheck size={11} /> Admin
        </p>
        {#each adminNav as group}
          <p class="px-3 pt-2 pb-1 text-[11px] font-medium text-sidebar-muted/70">{group.label}</p>
          <div class="space-y-0.5">
            {#each group.items as item}
              <a href={item.href} on:click={navigated} class="{linkBase} {isActive(item.href, path) ? linkActive : linkIdle}"
                 aria-current={isActive(item.href, path) ? 'page' : undefined}>
                <svelte:component this={item.icon} size={17} />
                {item.label}
              </a>
            {/each}
          </div>
        {/each}
      </nav>
    {/if}

    <section aria-label="Recent chats">
      <p class="px-3 mb-1.5 text-[10px] font-bold uppercase tracking-[0.12em] text-sidebar-muted/80">Recent chats</p>
      {#if $sessions.length === 0}
        <p class="px-3 py-2 text-xs text-sidebar-muted">No conversations yet.</p>
      {:else}
        <ul class="space-y-0.5">
          {#each $sessions as s (s.id)}
            {@const active = path === '/chat' && $activeSessionId === s.id}
            <li class="group relative flex items-center rounded-lg {active ? 'bg-sidebar-hover' : 'hover:bg-sidebar-hover'}">
              {#if renamingId === s.id}
                <input
                  bind:this={renameInput}
                  bind:value={renameValue}
                  on:keydown={(e) => { if (e.key === 'Enter') saveRename(); if (e.key === 'Escape') renamingId = null; }}
                  class="flex-1 min-w-0 h-8 mx-1 px-2 text-sm rounded-md bg-sidebar border border-sidebar-border text-sidebar-foreground outline-none focus:border-accent"
                  aria-label="Chat title"
                />
                <button on:click={saveRename} class="p-1.5 text-sidebar-muted hover:text-accent" aria-label="Save"><Check size={14} /></button>
                <button on:click={() => (renamingId = null)} class="p-1.5 mr-1 text-sidebar-muted hover:text-sidebar-foreground" aria-label="Cancel"><X size={14} /></button>
              {:else}
                <a href="/chat?s={s.id}" on:click={navigated}
                   class="flex-1 min-w-0 truncate px-3 py-2 text-[13px] {active ? 'text-sidebar-foreground font-medium' : 'text-sidebar-muted group-hover:text-sidebar-foreground'}">
                  {s.title || 'Untitled chat'}
                </a>
                <div class="flex items-center pr-1 opacity-100 lg:opacity-0 lg:group-hover:opacity-100 lg:focus-within:opacity-100 transition-opacity">
                  <button on:click={() => startRename(s.id, s.title)} class="p-1.5 rounded text-sidebar-muted hover:text-sidebar-foreground" aria-label="Rename chat">
                    <Pencil size={13} />
                  </button>
                  <button on:click={() => deleteChat(s.id, s.title)} class="p-1.5 rounded text-sidebar-muted hover:text-red-400" aria-label="Delete chat">
                    <Trash2 size={13} />
                  </button>
                </div>
              {/if}
            </li>
          {/each}
        </ul>
      {/if}
    </section>
  </div>

  <!-- Account -->
  <div class="relative border-t border-sidebar-border p-3 shrink-0">
    {#if accountOpen}
      <div class="fixed inset-0 z-10" on:click={() => (accountOpen = false)} role="presentation"></div>
      <div class="absolute left-3 right-3 bottom-full mb-2 z-20 rounded-xl border bg-card text-card-foreground shadow-xl py-1.5 overflow-hidden">
        <a href="/profile" on:click={navigated} class="flex items-center gap-2.5 px-3.5 py-2.5 text-sm hover:bg-muted">
          <UserRound size={16} class="text-muted-foreground" /> Profile & settings
        </a>
        <button on:click={isDark.toggle} class="w-full flex items-center gap-2.5 px-3.5 py-2.5 text-sm hover:bg-muted text-left">
          {#if $isDark}<Sun size={16} class="text-muted-foreground" /> Light mode{:else}<Moon size={16} class="text-muted-foreground" /> Dark mode{/if}
        </button>
        <div class="border-t my-1.5"></div>
        <button on:click={() => { accountOpen = false; dispatch('logout'); }}
          class="w-full flex items-center gap-2.5 px-3.5 py-2.5 text-sm text-destructive hover:bg-destructive/10 text-left">
          <LogOut size={16} /> Sign out
        </button>
      </div>
    {/if}
    <button on:click={() => (accountOpen = !accountOpen)}
      class="w-full flex items-center gap-3 p-2 rounded-lg hover:bg-sidebar-hover transition-colors text-left"
      aria-expanded={accountOpen} aria-haspopup="menu">
      <span class="w-9 h-9 rounded-full bg-accent text-accent-foreground text-xs font-extrabold flex items-center justify-center shrink-0">
        {initials}
      </span>
      <span class="flex-1 min-w-0 leading-tight">
        <span class="block text-sm font-semibold truncate">{$currentUser?.display_name || 'My account'}</span>
        <span class="block text-[11px] text-sidebar-muted truncate">{$currentUser?.email ?? ''}</span>
      </span>
      <ChevronsUpDown size={15} class="text-sidebar-muted shrink-0" />
    </button>
  </div>
</div>

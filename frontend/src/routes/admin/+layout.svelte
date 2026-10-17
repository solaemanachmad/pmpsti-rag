<script lang="ts">
  import { goto, afterNavigate } from '$app/navigation';
  import { page } from '$app/stores';
  import { currentUser, authReady } from '$lib/stores/auth';
  import { adminNav, findAdminItem, isNavActive } from '$lib/nav';
  import Modal from '$lib/components/ui/Modal.svelte';
  import { ChevronDown, Check, ShieldCheck } from 'lucide-svelte';

  // Desktop uses the sidebar; on mobile one button opens a sheet with every
  // admin section, so the menu can grow without crowding the screen.
  let switcherOpen = false;
  afterNavigate(() => (switcherOpen = false));

  $: if ($authReady) {
    if (!$currentUser) goto('/login');
    else if ($currentUser.role !== 'admin') goto('/chat');
  }

  $: path = $page.url.pathname;
  $: current = findAdminItem(path);
</script>

{#if $authReady && $currentUser?.role === 'admin'}
  <div class="h-full flex flex-col">
    <div class="lg:hidden shrink-0 border-b bg-card/60 px-3 py-2">
      <button on:click={() => (switcherOpen = true)} aria-haspopup="dialog" aria-expanded={switcherOpen}
        class="w-full flex items-center gap-3 h-11 px-3 rounded-lg border bg-card hover:bg-muted transition-colors text-left">
        {#if current}
          <span class="w-7 h-7 rounded-md bg-primary/10 text-primary flex items-center justify-center shrink-0">
            <svelte:component this={current.item.icon} size={15} />
          </span>
          <span class="flex-1 min-w-0 leading-tight">
            <span class="block text-[10px] font-bold uppercase tracking-[0.12em] text-muted-foreground">Admin · {current.group.label}</span>
            <span class="block text-sm font-semibold truncate">{current.item.label}</span>
          </span>
        {:else}
          <ShieldCheck size={16} class="text-muted-foreground" />
          <span class="flex-1 text-sm font-semibold">Admin</span>
        {/if}
        <ChevronDown size={16} class="text-muted-foreground shrink-0" />
      </button>
    </div>

    <div class="flex-1 overflow-y-auto">
      <div class="max-w-6xl mx-auto px-4 sm:px-6 py-6 sm:py-10">
        <slot />
      </div>
    </div>
  </div>

  <Modal open={switcherOpen} title="Admin" on:close={() => (switcherOpen = false)}>
    <nav aria-label="Admin sections" class="space-y-4 -mt-1">
      {#each adminNav as group}
        <div>
          <p class="text-[10px] font-bold uppercase tracking-[0.12em] text-muted-foreground mb-1.5 px-1">{group.label}</p>
          <ul class="space-y-1">
            {#each group.items as item}
              {@const active = isNavActive(item.href, path)}
              <li>
                <a href={item.href} aria-current={active ? 'page' : undefined}
                   class="flex items-center gap-3 h-12 px-3 rounded-xl transition-colors
                          {active ? 'bg-primary/10 text-primary font-semibold' : 'hover:bg-muted'}">
                  <span class="w-8 h-8 rounded-lg flex items-center justify-center shrink-0 {active ? 'bg-primary text-primary-foreground' : 'bg-muted text-muted-foreground'}">
                    <svelte:component this={item.icon} size={16} />
                  </span>
                  <span class="flex-1 text-sm">{item.label}</span>
                  {#if active}<Check size={16} />{/if}
                </a>
              </li>
            {/each}
          </ul>
        </div>
      {/each}
    </nav>
  </Modal>
{/if}

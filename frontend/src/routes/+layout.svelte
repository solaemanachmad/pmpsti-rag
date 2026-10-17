<script lang="ts">
  import '../app.css';
  import { onMount } from 'svelte';
  import { fade, fly } from 'svelte/transition';
  import { goto, afterNavigate } from '$app/navigation';
  import { page } from '$app/stores';
  import { Menu, SquarePen } from 'lucide-svelte';
  import { authStore, isLoggedIn, currentUser, authReady } from '$lib/stores/auth';
  import { refreshSessions, sessions } from '$lib/stores/chat';
  import { auth } from '$lib/api/client';
  import { isDark } from '$lib/theme';
  import Sidebar from '$lib/components/Sidebar.svelte';
  import Logo from '$lib/components/Logo.svelte';
  import ThemeToggle from '$lib/components/ThemeToggle.svelte';

  const publicRoutes = ['/', '/login', '/register', '/verify-email', '/forgot-password', '/reset-password'];

  let drawerOpen = false;

  onMount(async () => {
    isDark.sync();

    if ($currentUser) {
      authStore.setReady();
      auth.me().then(authStore.setUser).catch(() => {
        authStore.logout();
        goto('/login');
      });
    } else {
      try {
        authStore.setUser(await auth.me());
      } catch {
        authStore.setReady();
        if (!publicRoutes.includes($page.url.pathname)) goto('/login');
      }
    }
  });

  afterNavigate(() => (drawerOpen = false));

  // Load chat history once the user is known; clear it on sign-out.
  let loadedFor: number | null = null;
  $: if ($currentUser && loadedFor !== $currentUser.id) {
    loadedFor = $currentUser.id;
    refreshSessions();
  } else if (!$currentUser && loadedFor !== null) {
    loadedFor = null;
    sessions.set([]);
  }

  async function logout() {
    try { await auth.logout(); } catch { /* sign out locally even if the request fails */ }
    authStore.logout();
    goto('/');
  }

  $: isPublic = publicRoutes.includes($page.url.pathname);
</script>

<svelte:window on:keydown={(e) => drawerOpen && e.key === 'Escape' && (drawerOpen = false)} />

{#if isPublic}
  <slot />
{:else if !$authReady}
  <div class="flex h-dvh items-center justify-center bg-background">
    <div class="h-8 w-8 rounded-full border-[3px] border-muted border-t-primary animate-spin" aria-label="Loading"></div>
  </div>
{:else if $isLoggedIn}
  <div class="flex h-dvh overflow-hidden bg-background">
    <!-- Desktop sidebar -->
    <aside class="hidden lg:block w-72 shrink-0 border-r border-sidebar-border">
      <Sidebar on:logout={logout} />
    </aside>

    <!-- Mobile drawer -->
    {#if drawerOpen}
      <div class="lg:hidden fixed inset-0 z-50 bg-black/50 backdrop-blur-[2px]" transition:fade={{ duration: 150 }}
           on:click={() => (drawerOpen = false)} role="presentation"></div>
      <aside class="lg:hidden fixed inset-y-0 left-0 z-50 w-[85vw] max-w-[300px] shadow-2xl"
             transition:fly={{ x: -300, duration: 200, opacity: 1 }}>
        <Sidebar showClose on:close={() => (drawerOpen = false)} on:navigate={() => (drawerOpen = false)} on:logout={logout} />
      </aside>
    {/if}

    <div class="flex-1 min-w-0 flex flex-col">
      <!-- Mobile top bar -->
      <header class="lg:hidden h-14 shrink-0 flex items-center gap-1 px-2 border-b bg-card/80 backdrop-blur-md z-30">
        <button class="icon-btn" on:click={() => (drawerOpen = true)} aria-label="Open menu" aria-expanded={drawerOpen}>
          <Menu size={20} />
        </button>
        <a href="/chat" class="flex-1 min-w-0 px-1"><Logo compact /></a>
        <ThemeToggle />
        <a href="/chat" class="icon-btn" aria-label="New chat"><SquarePen size={18} /></a>
      </header>

      <main class="flex-1 min-h-0 overflow-hidden">
        <slot />
      </main>
    </div>
  </div>
{/if}

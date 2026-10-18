<script lang="ts">
  import { ArrowLeft, BookOpenCheck, Quote, Clock3 } from 'lucide-svelte';
  import Logo from './Logo.svelte';
  import ThemeToggle from './ThemeToggle.svelte';

  export let title: string;
  export let description = '';
</script>

<div class="min-h-dvh grid lg:grid-cols-[minmax(0,5fr)_minmax(0,6fr)] bg-background">
  <!-- Brand panel (desktop) -->
  <aside class="hidden lg:flex relative overflow-hidden flex-col justify-between p-10 xl:p-14 bg-sidebar text-sidebar-foreground">
    <div class="pointer-events-none absolute inset-0" aria-hidden="true">
      <div class="absolute -top-32 -left-32 w-96 h-96 rounded-full bg-primary/30 blur-3xl"></div>
      <div class="absolute bottom-0 right-0 w-80 h-80 rounded-full bg-accent/20 blur-3xl"></div>
    </div>
    <a href="/" class="relative"><Logo onDark /></a>
    <div class="relative max-w-md">
      <h2 class="text-3xl xl:text-4xl font-extrabold tracking-tight leading-tight">
        Your DTETI questions, <span class="text-accent">answered from official documents.</span>
      </h2>
      <ul class="mt-8 space-y-4 text-sm text-sidebar-muted">
        <li class="flex gap-3"><BookOpenCheck size={18} class="text-accent shrink-0" /> Grounded in published DTETI regulations and guides</li>
        <li class="flex gap-3"><Quote size={18} class="text-accent shrink-0" /> Every answer cites the documents it used</li>
        <li class="flex gap-3"><Clock3 size={18} class="text-accent shrink-0" /> Available any time, with your chat history saved</li>
      </ul>
    </div>
    <p class="relative text-xs text-sidebar-muted">DTETI · Universitas Gadjah Mada</p>
  </aside>

  <!-- Form -->
  <div class="flex flex-col min-h-dvh">
    <div class="flex items-center justify-between h-16 px-4 sm:px-6">
      <a href="/" class="lg:invisible"><Logo compact /></a>
      <ThemeToggle />
    </div>
    <div class="flex-1 flex items-center justify-center px-4 sm:px-6 pb-12">
      <div class="w-full max-w-sm">
        <h1 class="text-2xl font-extrabold tracking-tight">{title}</h1>
        {#if description}<p class="text-sm text-muted-foreground mt-1.5">{@html description}</p>{/if}
        <div class="mt-7">
          <slot />
        </div>
        <a href="/" class="mt-8 inline-flex items-center gap-1.5 text-xs font-medium text-muted-foreground hover:text-foreground">
          <ArrowLeft size={13} /> Back to home
        </a>
      </div>
    </div>
  </div>
</div>

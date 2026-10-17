<script lang="ts">
  import { onMount } from 'svelte';
  import { page } from '$app/stores';
  import { CheckCircle2, XCircle, Loader2 } from 'lucide-svelte';
  import { auth } from '$lib/api/client';
  import AuthShell from '$lib/components/AuthShell.svelte';

  let state: 'loading' | 'success' | 'error' = 'loading';
  let message = '';

  onMount(async () => {
    const token = $page.url.searchParams.get('token');
    if (!token) { state = 'error'; message = 'The verification token is missing from the link.'; return; }
    try {
      const result = await auth.verifyEmail(token);
      state = 'success';
      message = result?.message ?? 'Your email has been verified.';
    } catch {
      state = 'error';
      message = 'We couldn’t verify your email. The link may have expired — please try again.';
    }
  });

  $: title = state === 'loading' ? 'Verifying your email…' : state === 'success' ? 'Email verified' : 'Verification failed';
</script>

<svelte:head><title>Verify email · DTETI Answers</title></svelte:head>

<AuthShell {title}>
  <div class="card p-6 text-center">
    {#if state === 'loading'}
      <Loader2 size={34} class="animate-spin text-primary mx-auto" />
      <p class="text-sm text-muted-foreground mt-3">Just a moment…</p>
    {:else if state === 'success'}
      <div class="w-14 h-14 rounded-full bg-success/10 text-success flex items-center justify-center mx-auto"><CheckCircle2 size={26} /></div>
      <p class="text-sm text-muted-foreground mt-4">{message}</p>
      <a href="/login" class="btn-primary w-full mt-5">Sign in now</a>
    {:else}
      <div class="w-14 h-14 rounded-full bg-destructive/10 text-destructive flex items-center justify-center mx-auto"><XCircle size={26} /></div>
      <p class="text-sm text-muted-foreground mt-4">{message}</p>
      <a href="/register" class="btn-primary w-full mt-5">Register again</a>
    {/if}
  </div>
</AuthShell>

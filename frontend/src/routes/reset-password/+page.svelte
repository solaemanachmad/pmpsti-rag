<script lang="ts">
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import { page } from '$app/stores';
  import { auth } from '$lib/api/client';
  import { errorMessage } from '$lib/format';
  import AuthShell from '$lib/components/AuthShell.svelte';
  import Alert from '$lib/components/ui/Alert.svelte';
  import { Loader2 } from 'lucide-svelte';

  let token = '';
  let password = '';
  let confirm = '';
  let loading = false;
  let success = false;
  let error = '';

  onMount(() => {
    token = $page.url.searchParams.get('token') ?? '';
    if (!token) error = 'Reset token is missing. Please request a new link.';
  });

  async function submit() {
    if (password.length < 8) { error = 'Password must be at least 8 characters.'; return; }
    if (password !== confirm) { error = 'Passwords do not match.'; return; }
    loading = true; error = '';
    try {
      await auth.resetPassword(token, password);
      success = true;
      setTimeout(() => goto('/login'), 2500);
    } catch (e) {
      error = errorMessage(e) || 'Could not reset the password.';
    } finally {
      loading = false;
    }
  }
</script>

<svelte:head><title>Reset password · DTETI Answers</title></svelte:head>

<AuthShell title="Set a new password" description="Use at least 8 characters.">
  {#if success}
    <Alert variant="success">Password updated. Redirecting you to sign in…</Alert>
  {:else}
    <form on:submit|preventDefault={submit} class="space-y-4">
      {#if error}<Alert>{error}</Alert>{/if}
      <div>
        <label for="pw" class="label">New password</label>
        <input id="pw" bind:value={password} type="password" placeholder="At least 8 characters" class="input" autocomplete="new-password" required />
      </div>
      <div>
        <label for="confirm" class="label">Confirm password</label>
        <input id="confirm" bind:value={confirm} type="password" placeholder="Repeat the password" class="input" autocomplete="new-password" required />
      </div>
      <button type="submit" disabled={loading || !token} class="btn-primary w-full">
        {#if loading}<Loader2 size={16} class="animate-spin" />{/if}
        Save password
      </button>
    </form>
    <p class="text-sm text-center text-muted-foreground mt-6">
      <a href="/forgot-password" class="font-semibold text-primary hover:underline">Request a new link</a>
    </p>
  {/if}
</AuthShell>

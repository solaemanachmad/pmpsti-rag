<script lang="ts">
  import { auth } from '$lib/api/client';
  import { errorMessage } from '$lib/format';
  import AuthShell from '$lib/components/AuthShell.svelte';
  import Alert from '$lib/components/ui/Alert.svelte';
  import { Loader2 } from 'lucide-svelte';

  let email = '';
  let loading = false;
  let success = false;
  let error = '';

  async function submit() {
    if (!email.trim()) { error = 'Please enter your email.'; return; }
    loading = true; error = '';
    try {
      await auth.forgotPassword(email.trim());
      success = true;
    } catch (e) {
      error = errorMessage(e) || 'Could not send the reset email.';
    } finally {
      loading = false;
    }
  }
</script>

<svelte:head><title>Forgot password · DTETI Answers</title></svelte:head>

<AuthShell title="Forgot your password?" description="Enter your email and we’ll send you a reset link.">
  {#if success}
    <Alert variant="success">
      Email sent. Check your inbox and open the reset link — it expires in 1 hour.
    </Alert>
    <a href="/login" class="btn-outline w-full mt-4">Back to sign in</a>
  {:else}
    <form on:submit|preventDefault={submit} class="space-y-4">
      {#if error}<Alert>{error}</Alert>{/if}
      <div>
        <label for="email" class="label">Email</label>
        <input id="email" bind:value={email} type="email" placeholder="student@mail.ugm.ac.id" class="input" autocomplete="email" required />
      </div>
      <button type="submit" disabled={loading} class="btn-primary w-full">
        {#if loading}<Loader2 size={16} class="animate-spin" />{/if}
        Send reset link
      </button>
    </form>
    <p class="text-sm text-center text-muted-foreground mt-6">
      Remembered it? <a href="/login" class="font-semibold text-primary hover:underline">Sign in</a>
    </p>
  {/if}
</AuthShell>

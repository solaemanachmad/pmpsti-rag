<script lang="ts">
  import { createEventDispatcher, tick } from 'svelte';
  import { ArrowUp } from 'lucide-svelte';

  export let value = '';
  export let disabled = false;
  export let placeholder = 'Ask a question…';

  let el: HTMLTextAreaElement;
  const dispatch = createEventDispatcher<{ submit: string }>();

  function submit() {
    const text = value.trim();
    if (!text || disabled) return;
    dispatch('submit', text);
    tick().then(resize);
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) {
      e.preventDefault();
      submit();
    }
  }

  function resize() {
    if (!el) return;
    el.style.height = 'auto';
    el.style.height = Math.min(el.scrollHeight, 180) + 'px';
  }

  export function focus() { el?.focus(); }
</script>

<form on:submit|preventDefault={submit}
  class="flex items-end gap-2 rounded-2xl border bg-card pl-4 pr-2 py-2 shadow-soft
         focus-within:border-ring focus-within:ring-2 focus-within:ring-ring/20 transition-shadow">
  <textarea
    bind:this={el}
    bind:value
    on:keydown={onKeydown}
    on:input={resize}
    {placeholder}
    {disabled}
    rows="1"
    aria-label="Message"
    class="flex-1 resize-none bg-transparent text-[15px] leading-6 outline-none py-1.5
           placeholder:text-muted-foreground disabled:cursor-not-allowed max-h-44"
  ></textarea>
  <button
    type="submit"
    disabled={!value.trim() || disabled}
    aria-label="Send"
    class="shrink-0 w-9 h-9 rounded-xl flex items-center justify-center bg-primary text-primary-foreground
           disabled:bg-muted disabled:text-muted-foreground transition-colors"
  >
    <ArrowUp size={18} strokeWidth={2.5} />
  </button>
</form>

<script lang="ts" context="module">
  import type { Category } from '$lib/api/client';

  /** A top-level category with its children; `total` = own documents + children's. */
  export interface CategoryNode {
    cat: Category;
    count: number;
    total: number;
    children: { cat: Category; count: number }[];
  }
</script>

<script lang="ts">
  import { ChevronDown, FileText, Folder, TriangleAlert } from 'lucide-svelte';

  /** '' = all documents, 'none' = uncategorized, otherwise a category id as a string */
  export let selected = '';
  export let nodes: CategoryNode[] = [];
  export let allCount = 0;
  export let uncategorizedCount = 0;

  // Parents start expanded (the list is short); the admin can fold them.
  let collapsed: Record<number, boolean> = {};

  function toggle(id: number) {
    collapsed = { ...collapsed, [id]: !collapsed[id] };
  }

  const item = 'flex-1 min-w-0 flex items-center gap-2 px-2 py-1.5 rounded-md text-sm text-left transition-colors';
  function itemClass(active: boolean): string {
    return `${item} ${active ? 'bg-primary/10 text-primary font-semibold' : 'text-foreground hover:bg-muted'}`;
  }
</script>

<nav aria-label="Document categories" class="text-sm">
  <ul class="space-y-0.5">
    <li>
      <button type="button" class="{itemClass(selected === '')} w-full" aria-current={selected === '' ? 'true' : undefined}
        on:click={() => (selected = '')}>
        <FileText size={14} class="shrink-0 opacity-70" />
        <span class="flex-1 truncate">All documents</span>
        <span class="text-xs tabular-nums text-muted-foreground">{allCount}</span>
      </button>
    </li>

    {#each nodes as node (node.cat.id)}
      {@const key = String(node.cat.id)}
      {@const open = !collapsed[node.cat.id]}
      <li>
        <div class="flex items-center gap-0.5">
          {#if node.children.length}
            <button type="button" class="w-6 h-7 flex items-center justify-center rounded-md text-muted-foreground hover:bg-muted shrink-0"
              aria-expanded={open} aria-label="{open ? 'Collapse' : 'Expand'} {node.cat.name}" on:click={() => toggle(node.cat.id)}>
              <ChevronDown size={14} class="transition-transform {open ? '' : '-rotate-90'}" />
            </button>
          {:else}
            <span class="w-6 shrink-0" aria-hidden="true"></span>
          {/if}
          <button type="button" class={itemClass(selected === key)} aria-current={selected === key ? 'true' : undefined}
            on:click={() => (selected = key)}>
            <Folder size={14} class="shrink-0 opacity-70" />
            <span class="flex-1 truncate" title={node.cat.name}>{node.cat.name}</span>
            <span class="text-xs tabular-nums text-muted-foreground">{node.total}</span>
          </button>
        </div>
        {#if open && node.children.length}
          <ul class="ml-6 pl-2 border-l space-y-0.5 mt-0.5">
            {#each node.children as child (child.cat.id)}
              {@const ckey = String(child.cat.id)}
              <li>
                <button type="button" class="{itemClass(selected === ckey)} w-full" aria-current={selected === ckey ? 'true' : undefined}
                  on:click={() => (selected = ckey)}>
                  <span class="flex-1 truncate" title={child.cat.name}>{child.cat.name}</span>
                  <span class="text-xs tabular-nums text-muted-foreground">{child.count}</span>
                </button>
              </li>
            {/each}
          </ul>
        {/if}
      </li>
    {/each}

    <li class="pt-2 mt-2 border-t">
      <button type="button" class="{itemClass(selected === 'none')} w-full" aria-current={selected === 'none' ? 'true' : undefined}
        on:click={() => (selected = 'none')}>
        <TriangleAlert size={14} class="shrink-0 text-warning" />
        <span class="flex-1 truncate">Uncategorized</span>
        <span class="text-xs tabular-nums {uncategorizedCount ? 'text-warning font-semibold' : 'text-muted-foreground'}">{uncategorizedCount}</span>
      </button>
    </li>
  </ul>
</nav>

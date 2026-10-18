import type { ComponentType } from 'svelte';
import {
  MessageSquare, KeyRound, LayoutDashboard, Users, FileText, ScrollText, HelpCircle, FolderSync
} from 'lucide-svelte';

export interface NavItem {
  href: string;
  label: string;
  icon: ComponentType;
}

export interface NavGroup {
  label: string;
  items: NavItem[];
}

export const mainNav: NavItem[] = [
  { href: '/chat', label: 'Chat', icon: MessageSquare },
  { href: '/keys', label: 'API Keys', icon: KeyRound }
];

// Single source for the admin menu: the desktop sidebar and the mobile
// section switcher both render from this, so a new page is one entry here.
export const adminNav: NavGroup[] = [
  {
    label: 'Analytics',
    items: [
      { href: '/admin',      label: 'Overview',   icon: LayoutDashboard },
      { href: '/admin/logs', label: 'Query Logs', icon: ScrollText }
    ]
  },
  {
    label: 'Content',
    items: [
      { href: '/admin/documents', label: 'Documents', icon: FileText },
      { href: '/admin/sources',   label: 'Sources',   icon: FolderSync },
      { href: '/admin/faq',       label: 'FAQ',       icon: HelpCircle }
    ]
  },
  {
    label: 'People',
    items: [
      { href: '/admin/users', label: 'Users', icon: Users }
    ]
  }
];

/** Section roots like `/admin` only match exactly; other items also match their sub-pages. */
export function isNavActive(href: string, path: string): boolean {
  if (href === '/admin') return path === '/admin';
  return path === href || path.startsWith(href + '/');
}

export function findAdminItem(path: string): { group: NavGroup; item: NavItem } | null {
  for (const group of adminNav) {
    const item = group.items.find(i => isNavActive(i.href, path));
    if (item) return { group, item };
  }
  return null;
}

// Always call the backend through this site's own /api route: the Vite dev proxy
// locally, and src/routes/api/[...path]/+server.ts on Vercel. Calling the Render
// backend directly made the login cookie a third-party cookie, which Safari/iOS
// drop ("Token tidak ditemukan" after every login).
const BASE = '/api';

async function request<T>(path: string, options: RequestInit = {}): Promise<T> {
  const headers: Record<string, string> = {
    'Content-Type': 'application/json',
    ...(options.headers as Record<string, string> ?? {})
  };
  const res = await fetch(`${BASE}${path}`, {
    ...options,
    headers,
    credentials: 'include',  // kirim httpOnly cookie otomatis
  });
  const json = await res.json();
  if (!res.ok) throw new Error(json?.error?.message ?? json?.error ?? `HTTP ${res.status}`);
  return (json.data ?? json) as T;
}

// ── Auth ──
export const auth = {
  register: (email: string, password: string, display_name?: string) =>
    request<{ message: string }>('/auth/register', {
      method: 'POST',
      body: JSON.stringify({ email, password, display_name })
    }),
  login: (email: string, password: string) =>
    request<AuthResponse>('/auth/login', {
      method: 'POST',
      body: JSON.stringify({ email, password })
    }),
  me: () => request<UserPublic>('/auth/me'),
  logout: () => request<{ message: string }>('/auth/logout', { method: 'POST' }),
  verifyEmail: (token: string) =>
    request<{ message: string }>(`/auth/verify/${encodeURIComponent(token)}`),
  forgotPassword: (email: string) =>
    request<{ message: string }>('/auth/forgot-password', {
      method: 'POST',
      body: JSON.stringify({ email }),
    }),
  resetPassword: (token: string, password: string) =>
    request<{ message: string }>('/auth/reset-password', {
      method: 'POST',
      body: JSON.stringify({ token, password }),
    }),
  updateProfile: (data: { display_name?: string; email?: string }) =>
    request('/auth/me', { method: 'PATCH', body: JSON.stringify(data) }),
  updatePassword: (current_password: string, new_password: string) =>
    request('/auth/me/password', {
      method: 'PATCH',
      body: JSON.stringify({ current_password, new_password })
    })
};

// ── Chat ──
export const chat = {
  ask: (query: string, session_id?: string, category_filter?: string, top_k = 5) =>
    request<AskResponse>('/ask', {
      method: 'POST',
      body: JSON.stringify({ query, session_id, category_filter, top_k, search_mode: 'hybrid' })
    }),
  sessions: () => request<SessionListItem[]>('/sessions'),
  getSession: (id: string) => request<ChatSession>(`/sessions/${id}`),
  deleteSession: (id: string) => request(`/sessions/${id}`, { method: 'DELETE' }),
  renameSession: (id: string, title: string) =>
    request(`/sessions/${id}/title`, { method: 'PATCH', body: JSON.stringify({ title }) }),
  categories: () => request<string[]>('/categories')
};

// ── API Keys ──
export const keys = {
  list: () => request<ApiKeyInfo[]>('/keys'),
  create: (name: string, permissions?: string[], rate_limit?: number, expires_at?: string) =>
    request<CreateApiKeyResponse>('/keys', {
      method: 'POST',
      body: JSON.stringify({ name, permissions, rate_limit, expires_at })
    }),
  revoke: (id: number) => request(`/keys/${id}`, { method: 'DELETE' })
};

// ── Admin ──
export const admin = {
  stats: () =>
    request<QueryLogStats>('/admin/stats'),

  // Users
  users: () =>
    request<AdminUser[]>('/admin/users'),
  /** @deprecated use users() */
  listUsers: () =>
    request<AdminUser[]>('/admin/users'),
  setRole: (id: number, role: string) =>
    request<{ message: string }>(`/admin/users/${id}/role`, {
      method: 'PATCH',
      body: JSON.stringify({ role })
    }),
  toggleUser: (id: number, is_active: boolean) =>
    request<{ message: string }>(`/admin/users/${id}/active`, {
      method: 'PATCH',
      body: JSON.stringify({ is_active })
    }),
  /** @deprecated use toggleUser() */
  toggleActive: (id: number, is_active: boolean) =>
    request<{ message: string }>(`/admin/users/${id}/active`, {
      method: 'PATCH',
      body: JSON.stringify({ is_active })
    }),

  verifyUser: (id: number) =>
    request<{ message: string }>(`/admin/users/${id}/verify`, { method: 'PATCH' }),

  deleteUser: (id: number) =>
    request<{ message: string }>(`/admin/users/${id}`, { method: 'DELETE' }),

  // Create user (admin)
  createUser: (email: string, display_name?: string, role = 'user') =>
    request<AdminUser>('/admin/users', {
      method: 'POST',
      body: JSON.stringify({ email, display_name, role })
    }),

  // Sessions
  listSessions: () =>
    request<AdminSession[]>('/admin/sessions'),
  deleteSession: (id: string) =>
    request(`/admin/sessions/${encodeURIComponent(id)}`, { method: 'DELETE' }),

  // Documents
  documents: () =>
    request<AdminDocument[]>('/admin/documents'),
  /** @deprecated use documents() */
  listDocuments: () =>
    request<AdminDocument[]>('/admin/documents'),
  deleteDocument: (document_id: string) =>
    request<{ deleted_chunks: number }>(
      `/admin/documents/${encodeURIComponent(document_id)}`,
      { method: 'DELETE' }
    ),
  updateDocument: (document_id: string, payload: { title: string; category: string; subcategory: string }) =>
    request<{ updated_chunks: number }>(
      `/admin/documents/${encodeURIComponent(document_id)}`,
      { method: 'PATCH', body: JSON.stringify(payload) }
    ),

  // Document ingest
  ingestUrl: (payload: { url: string; title?: string; category?: string; subcategory?: string; chunk_strategy?: string }) =>
    request<{ document_id: string; chunks: number; title: string }>(
      '/admin/documents/ingest-url',
      { method: 'POST', body: JSON.stringify(payload) }
    ),

  // Chunk operations
  listChunks: (document_id: string) =>
    request<AdminChunk[]>(`/admin/chunks?document_id=${encodeURIComponent(document_id)}`),
  deleteChunk: (chunk_id: number) =>
    request<{ deleted: number }>(`/admin/chunks/${chunk_id}`, { method: 'DELETE' }),
  updateChunk: (chunk_id: number, content: string) =>
    request<{ updated: boolean }>(`/admin/chunks/${chunk_id}`, {
      method: 'PUT',
      body: JSON.stringify({ content })
    }),

  // Query logs
  queryLogs: (limit = 50, offset = 0) =>
    request<{ logs: AdminQueryLog[]; total: number }>(`/admin/logs?limit=${limit}&offset=${offset}`)
};

// ── Streaming ask ──
export function askStream(
  query: string,
  session_id: string | undefined,
  onChunk: (text: string) => void,
  onDone: (sources: SourceRef[], session_id: string, ms: number) => void,
  onError: (err: string) => void
): () => void {
  const ctrl = new AbortController();
  let cancelled = false;

  (async () => {
    try {
      const res = await fetch(`${BASE}/ask`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        credentials: 'include',
        body: JSON.stringify({ query, session_id, search_mode: 'hybrid', top_k: 5 }),
        signal: ctrl.signal
      });

      if (!res.ok) {
        const j = await res.json().catch(() => ({}));
        onError(j?.error ?? `HTTP ${res.status}`);
        return;
      }

      const json = await res.json();
      const data: AskResponse = json.data ?? json;
      if (cancelled) return;

      const words = data.answer.split(' ');
      for (let i = 0; i < words.length; i++) {
        if (cancelled) return;
        await new Promise(r => setTimeout(r, 25));
        onChunk((i === 0 ? '' : ' ') + words[i]);
      }
      onDone(data.sources ?? [], data.session_id ?? '', data.search_time_ms ?? 0);
    } catch (e: unknown) {
      if (e instanceof Error && e.name !== 'AbortError' && !cancelled) onError(String(e));
    }
  })();

  return () => { cancelled = true; ctrl.abort(); };
}

// ══════════════════════════════════════════════════════════════════
//  TYPES
// ══════════════════════════════════════════════════════════════════

export interface AuthResponse {
  token: string;
  token_type: string;
  expires_in: number;
  user: UserPublic;
}
export interface UserPublic {
  id: number;
  email: string;
  display_name: string;
  role: string;
  created_at: string;
}
export interface AskResponse {
  answer: string;
  session_id: string;
  sources: SourceRef[];
  search_time_ms: number;
}
export interface SourceRef {
  title: string;
  snippet: string;
  source_url: string;
  category: string;
  subcategory?: string;
  score: number;
}
export interface SessionListItem {
  id: string;
  title: string;
  updated_at: string;
  message_count: number;
}
export interface ChatSession {
  id: string;
  user_id: number;
  title: string;
  messages: ChatMessage[];
  created_at: string;
  updated_at: string;
}
export interface ChatSource {
  title: string;
  snippet: string;
  source_url: string;
  category: string;
  subcategory?: string;
  score: number;
}
export interface ChatMessage {
  role: 'user' | 'assistant';
  content: string;
  created_at: string;
  sources?: ChatSource[];
}
export interface ApiKeyInfo {
  id: number;
  key_prefix: string;
  name: string;
  permissions: string[];
  rate_limit: number;
  is_active: boolean;
  last_used_at: string | null;
  created_at: string;
  expires_at: string | null;
}
export interface CreateApiKeyResponse {
  id: number;
  key: string;
  key_prefix: string;
  name: string;
}
export interface QueryLogStats {
  total_queries: number;
  unique_queries: number;
  avg_results: number;
  avg_search_time_ms: number;
  zero_result_queries: number;
  language_distribution: [string, number][];
  domain_distribution: [string, number][];
  queries_per_day: [string, number][];
  top_queries: [string, number][];
}

// Admin types
export interface AdminUser {
  id: number;
  email: string;
  display_name: string;
  role: string;
  is_active: boolean;
  email_verified: boolean;
  created_at: string;
}
export interface AdminSession {
  id: string;
  user_id: number;
  user_email: string;
  title: string;
  message_count: number;
  updated_at: string;
}
export interface AdminDocument {
  document_id: string;
  title: string;
  document_type: string;
  category: string;
  subcategory: string;
  source_url: string;
  chunk_count: number;
}
export interface IngestResponse {
  document_id: string;
  chunks: number;
  title: string;
}

export interface AdminChunk {
  id: number;
  chunk_index: number;
  content: string;
  title: string;
  category: string;
  subcategory: string;
  source_url: string;
  created_at: string;
}

export interface AdminQueryLog {
  id: number;
  query_text: string;
  detected_language: string;
  num_results: number;
  search_time_ms: number;
  user_id: number | null;
  session_id: string | null;
  created_at: string;
}

// ── FAQ ──
export const faq = {
  /** Public: ambil daftar pertanyaan FAQ (pinned + top queries) */
  list: () => request<string[]>('/faq'),

  /** Admin: list pinned questions */
  pins: () => request<FaqPin[]>('/admin/faq/pins'),

  /** Admin: top queries dari logs untuk dipilih di-pin */
  topQueries: () => request<TopQuery[]>('/admin/faq/top'),

  /** Admin: pin sebuah pertanyaan */
  pin: (question: string) =>
    request<{ id: number }>('/admin/faq/pins', {
      method: 'POST',
      body: JSON.stringify({ question }),
    }),

  /** Admin: unpin */
  unpin: (id: number) =>
    request<{ message: string }>(`/admin/faq/pins/${id}`, { method: 'DELETE' }),
};

export interface FaqPin {
  id: number;
  question: string;
  sort_order: number;
  created_at: string;
}

export interface TopQuery {
  query_text: string;
  count: number;
  last_asked: string;
}

// ── Public ask (guest, no auth) ──
export interface GuestAskResponse {
  answer: string;
  sources: SourceRef[];
  search_time_ms: number;
  questions_used: number;
  questions_left: number;
}

export function askPublic(
  query: string,
  guestToken: string,
  onChunk: (text: string) => void,
  onDone: (res: GuestAskResponse) => void,
  onError: (err: string) => void
): () => void {
  const ctrl = new AbortController();
  let cancelled = false;

  (async () => {
    try {
      const res = await fetch(`${BASE}/ask_public`, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          'X-Guest-Token': guestToken
        },
        body: JSON.stringify({ query }),
        signal: ctrl.signal
      });

      if (res.status === 429) {
        onError('__quota_exceeded__');
        return;
      }

      if (!res.ok) {
        const j = await res.json().catch(() => ({}));
        onError(j?.error ?? `HTTP ${res.status}`);
        return;
      }

      const json = await res.json();
      const data: GuestAskResponse = json.data ?? json;
      if (cancelled) return;

      const words = data.answer.split(' ');
      for (let i = 0; i < words.length; i++) {
        if (cancelled) return;
        await new Promise(r => setTimeout(r, 25));
        onChunk((i === 0 ? '' : ' ') + words[i]);
      }

      onDone(data);
    } catch (e: unknown) {
      if (e instanceof Error && e.name !== 'AbortError' && !cancelled) onError(String(e));
    }
  })();

  return () => { cancelled = true; ctrl.abort(); };
}

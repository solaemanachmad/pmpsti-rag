import type { RequestHandler } from '@sveltejs/kit';
import { env } from '$env/dynamic/private';
import { sameOriginWrite } from '$lib/server/same-origin';

const BACKEND = 'https://pmpsti-rag.onrender.com';

// Vercel function time limit for this proxy. Answers can take a while (Render's free
// plan needs ~50 s to wake up, plus the LLM), and the default limit is shorter.
// 60 s is the maximum on Vercel's Hobby plan.
export const config = { maxDuration: 60 };

// Request headers passed through to the backend.
const REQUEST_HEADERS = ['Content-Type', 'Accept', 'Authorization', 'X-Guest-Token', 'X-Api-Key', 'Cookie'];

// Response headers that must not be copied: hop-by-hop headers, and headers
// that describe the backend's wire encoding (fetch already decompressed the
// body, so the runtime recomputes these). Set-Cookie is handled separately.
const SKIP_RESPONSE_HEADERS = new Set([
  'connection',
  'keep-alive',
  'transfer-encoding',
  'te',
  'trailer',
  'upgrade',
  'proxy-authenticate',
  'proxy-authorization',
  'content-encoding',
  'content-length',
  'set-cookie',
]);

async function proxy(request: Request, url: URL, clientAddress: string): Promise<Response> {
  // Use the path exactly as the browser sent it (still percent-encoded). The decoded
  // route param turns an encoded id like "chapter7%2Fgap%2Findex" back into real
  // slashes, which no backend route matches (404 with an empty body).
  const target = `${BACKEND}${url.pathname}${url.search}`;

  const headers = new Headers();
  for (const h of REQUEST_HEADERS) {
    const val = request.headers.get(h);
    if (val) headers.set(h, val);
  }
  // Every request reaches the backend from a Vercel address, so tell it who the visitor is
  // (login and question limits are per visitor). The backend trusts X-Real-IP only together
  // with the shared PROXY_SECRET (server-side env var on Vercel and Render, never sent to the
  // browser). Headers of the same name sent by the browser are not in REQUEST_HEADERS, so
  // they are never passed on.
  if (env.PROXY_SECRET && clientAddress) {
    headers.set('X-Real-IP', clientAddress);
    headers.set('X-Proxy-Secret', env.PROXY_SECRET);
  }

  const body = request.method !== 'GET' && request.method !== 'HEAD'
    ? await request.arrayBuffer()
    : undefined;

  const res = await fetch(target, {
    method: request.method,
    headers,
    body,
    redirect: 'manual',
  });

  const resHeaders = new Headers();
  res.headers.forEach((value, key) => {
    if (!SKIP_RESPONSE_HEADERS.has(key)) resHeaders.set(key, value);
  });
  // Forward every cookie unchanged. The backend sets no Domain attribute, so the
  // browser stores it for this frontend's host: a first-party cookie that
  // Safari/iOS and other third-party-cookie blockers keep.
  for (const cookie of res.headers.getSetCookie()) {
    resHeaders.append('Set-Cookie', cookie);
  }

  // Stream the body instead of buffering it; null-body statuses get no body.
  const noBody = request.method === 'HEAD' || [101, 204, 205, 304].includes(res.status);
  return new Response(noBody ? null : res.body, {
    status: res.status,
    statusText: res.statusText,
    headers: resHeaders,
  });
}

const handler: RequestHandler = ({ request, url, getClientAddress }) => {
  if (!sameOriginWrite(request.method, request.headers.get('Origin'), url.origin)) {
    return new Response(JSON.stringify({ success: false, error: 'Permintaan dari situs lain ditolak', code: 403 }), {
      status: 403,
      headers: { 'Content-Type': 'application/json' },
    });
  }
  let clientAddress = '';
  try { clientAddress = getClientAddress(); } catch { /* not available in this environment */ }
  return proxy(request, url, clientAddress);
};

export const GET = handler;
export const POST = handler;
export const PATCH = handler;
export const DELETE = handler;
export const PUT = handler;

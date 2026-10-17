import type { RequestHandler } from '@sveltejs/kit';

const BACKEND = 'https://pmpsti-rag.onrender.com';

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

async function proxy(request: Request, path: string, search: string): Promise<Response> {
  const url = `${BACKEND}/api/${path}${search}`;

  const headers = new Headers();
  for (const h of REQUEST_HEADERS) {
    const val = request.headers.get(h);
    if (val) headers.set(h, val);
  }

  const body = request.method !== 'GET' && request.method !== 'HEAD'
    ? await request.arrayBuffer()
    : undefined;

  const res = await fetch(url, {
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

const handler: RequestHandler = ({ request, params, url }) =>
  proxy(request, params.path ?? '', url.search);

export const GET = handler;
export const POST = handler;
export const PATCH = handler;
export const DELETE = handler;
export const PUT = handler;

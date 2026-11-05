// CSRF guard for the /api proxy. The login cookie is SameSite=Lax, and this is the second
// layer: a request that changes data is refused when the browser says it comes from another
// site's page. Browsers always send `Origin` on such requests; tools (curl, scripts) send none.
// The proxy does not pass `Origin` on, so the backend cannot do this check for proxied requests.

const READ_ONLY = new Set(['GET', 'HEAD', 'OPTIONS']);

/** True when the request may be passed to the backend. `ownOrigin` is this frontend's own
 *  origin (e.g. https://pmpsti-rag.vercel.app or a Vercel Preview address). */
export function sameOriginWrite(method: string, origin: string | null, ownOrigin: string): boolean {
  if (READ_ONLY.has(method.toUpperCase())) return true;
  if (origin === null) return true;
  return origin.toLowerCase() === ownOrigin.toLowerCase();
}

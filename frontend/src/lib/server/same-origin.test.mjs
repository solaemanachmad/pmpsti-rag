// Run: node --experimental-strip-types --test src/lib/server/same-origin.test.mjs
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { sameOriginWrite } from './same-origin.ts';

const own = 'https://pmpsti-rag.vercel.app';

test('our own pages may change data', () => {
  assert.equal(sameOriginWrite('POST', own, own), true);
  assert.equal(sameOriginWrite('delete', 'https://PMPSTI-RAG.vercel.app', own), true);
  const preview = 'https://pmpsti-rag-git-fix-x.vercel.app';
  assert.equal(sameOriginWrite('PATCH', preview, preview), true);
});

test('tools without an Origin header are not blocked', () => {
  assert.equal(sameOriginWrite('POST', null, own), true);
});

test("another site's page may not change data", () => {
  assert.equal(sameOriginWrite('POST', 'https://evil.example', own), false);
  assert.equal(sameOriginWrite('PUT', 'null', own), false);
  assert.equal(sameOriginWrite('POST', 'https://pmpsti-rag.vercel.app.evil.example', own), false);
  // a Preview page may not write through production, and the other way round
  assert.equal(sameOriginWrite('POST', 'https://pmpsti-rag-git-fix-x.vercel.app', own), false);
});

test('reading is never blocked', () => {
  assert.equal(sameOriginWrite('GET', 'https://evil.example', own), true);
  assert.equal(sameOriginWrite('HEAD', 'https://evil.example', own), true);
});

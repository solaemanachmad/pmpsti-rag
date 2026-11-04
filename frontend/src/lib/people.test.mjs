// Run: node --experimental-strip-types --test src/lib/people.test.mjs
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { personFromTitle, personKey } from './people.ts';

test('reads the name from a person page title', () => {
  assert.equal(personFromTitle('Dosen — Sunu Wibirama — Penelitian'), 'Sunu Wibirama');
  assert.equal(personFromTitle('Dosen — Guntur D. Putra'), 'Guntur D. Putra');
  assert.equal(personFromTitle('dosen — Sunu  Wibirama — Mata Kuliah'), 'Sunu Wibirama');
  assert.equal(personFromTitle('Tendik — Budi — Kontak'), 'Budi');
});

test('other titles have no person', () => {
  assert.equal(personFromTitle('Lampiran F — Pengelolaan Program Studi'), null);
  assert.equal(personFromTitle('Tenaga Pendidik'), null);
  assert.equal(personFromTitle('Dosen - Sunu Wibirama'), null); // a plain hyphen is not a separator
  // website pages: "<Page> – <site>" with an en dash
  assert.equal(personFromTitle('Dosen – jteti ft ugm (v20261018)'), null);
  assert.equal(personFromTitle('Pengelola – jteti ft ugm (v20261018)'), null);
  assert.equal(personFromTitle('Dosen — '), null);
  assert.equal(personFromTitle(''), null);
  assert.equal(personFromTitle(null), null);
});

test('the same person with different spelling details is one group', () => {
  assert.equal(personKey('Guntur D. Putra'), personKey('guntur d putra'));
  assert.notEqual(personKey('Sunu Wibirama'), personKey('Guntur D. Putra'));
});

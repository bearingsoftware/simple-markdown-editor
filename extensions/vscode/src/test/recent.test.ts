import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import { keepExisting, MAX_BYTES, MAX_ENTRIES, parseRecent, type RecentDeps, readRecent } from '../recent';

const LIST = 'C:\\Users\\craig\\AppData\\Local\\SimpleMarkdownEditor\\recent.json';

function json(paths: unknown): string {
  return JSON.stringify({ paths });
}

function bytes(text: string): Uint8Array {
  return new TextEncoder().encode(text);
}

test('parseRecent keeps order and drops entries that are not local drive paths', () => {
  const text = json([
    'C:\\notes\\first.md',
    'relative\\second.md',
    '\\\\server\\share\\third.md',
    '\\\\?\\C:\\fourth.md',
    '\\\\.\\C:\\fifth.md',
    42,
    null,
    'D:\\sixth.md',
    'C:\\bad\0name.md',
    'C:relative-drive.md',
  ]);
  assert.deepEqual(parseRecent(text), ['C:\\notes\\first.md', 'D:\\sixth.md']);
});

test('parseRecent caps the list at ten entries', () => {
  const paths = Array.from({ length: 15 }, (_, i) => `C:\\notes\\${i}.md`);
  const got = parseRecent(json(paths));
  assert.equal(got.length, MAX_ENTRIES);
  assert.deepEqual(got, paths.slice(0, MAX_ENTRIES));
});

test('parseRecent returns nothing for input that is not the expected shape', () => {
  const cases = ['', 'not json', '[]', 'null', '{"paths": "C:\\\\a.md"}', '{"other": []}'];
  for (const text of cases) {
    assert.deepEqual(parseRecent(text), [], JSON.stringify(text));
  }
});

test('readRecent asks for one byte more than the limit', async () => {
  let asked = -1;
  const deps: RecentDeps = {
    readBounded: async (_path, limit) => {
      asked = limit;
      return bytes(json(['C:\\a.md']));
    },
  };
  await readRecent(LIST, deps);
  assert.equal(asked, MAX_BYTES + 1);
});

test('readRecent refuses content over the limit', async () => {
  const deps: RecentDeps = {
    readBounded: async () => new Uint8Array(MAX_BYTES + 1).fill(0x20),
  };
  assert.deepEqual(await readRecent(LIST, deps), []);
});

test('readRecent accepts content exactly at the limit', async () => {
  const pad = ' '.repeat(MAX_BYTES - json(['C:\\a.md']).length);
  const text = json(['C:\\a.md']) + pad;
  assert.equal(bytes(text).byteLength, MAX_BYTES);
  const deps: RecentDeps = { readBounded: async () => bytes(text) };
  assert.deepEqual(await readRecent(LIST, deps), ['C:\\a.md']);
});

test('readRecent returns nothing for a missing list', async () => {
  const deps: RecentDeps = { readBounded: async () => undefined };
  assert.deepEqual(await readRecent(LIST, deps), []);
});

test('readRecent returns nothing when the read itself fails', async () => {
  const deps: RecentDeps = {
    readBounded: async () => {
      throw new Error('EBUSY');
    },
  };
  assert.deepEqual(await readRecent(LIST, deps), []);
});

test('keepExisting drops missing files and keeps order', async () => {
  const present = new Set(['C:\\a.md', 'C:\\c.md']);
  const stat = async (path: string) => (present.has(path) ? { size: 1, isFile: true } : undefined);
  assert.deepEqual(await keepExisting(['C:\\a.md', 'C:\\b.md', 'C:\\c.md'], stat), ['C:\\a.md', 'C:\\c.md']);
});

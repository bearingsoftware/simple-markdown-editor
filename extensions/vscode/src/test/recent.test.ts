import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import { keepExisting, MAX_BYTES, MAX_ENTRIES, type RecentDeps, parseRecent, readRecent } from '../recent';

const LIST = 'C:\\Users\\craig\\AppData\\Local\\SimpleMarkdownEditor\\recent.json';

function json(paths: unknown): string {
  return JSON.stringify({ paths });
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

test('readRecent refuses a file over the size bound without reading it', async () => {
  let read = false;
  const deps: RecentDeps = {
    stat: async () => ({ size: MAX_BYTES + 1, isFile: true }),
    read: async () => {
      read = true;
      return json(['C:\\a.md']);
    },
  };
  assert.deepEqual(await readRecent(LIST, deps), []);
  assert.equal(read, false);
});

test('readRecent returns nothing for a missing list', async () => {
  const deps: RecentDeps = {
    stat: async () => undefined,
    read: async () => {
      throw new Error('should not read');
    },
  };
  assert.deepEqual(await readRecent(LIST, deps), []);
});

test('readRecent returns nothing when the read itself fails', async () => {
  const deps: RecentDeps = {
    stat: async () => ({ size: 10, isFile: true }),
    read: async () => {
      throw new Error('EBUSY');
    },
  };
  assert.deepEqual(await readRecent(LIST, deps), []);
});

test('keepExisting drops missing files and keeps order', async () => {
  const present = new Set(['C:\\a.md', 'C:\\c.md']);
  const stat: RecentDeps['stat'] = async (path) =>
    present.has(path) ? { size: 1, isFile: true } : undefined;
  assert.deepEqual(await keepExisting(['C:\\a.md', 'C:\\b.md', 'C:\\c.md'], stat), ['C:\\a.md', 'C:\\c.md']);
});

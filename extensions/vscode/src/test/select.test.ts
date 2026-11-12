import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import { filterMarkdownFiles, isMarkdownName } from '../select';

test('isMarkdownName accepts .md and .markdown in any case', () => {
  const cases: Array<[string, boolean]> = [
    ['C:\\notes\\a.md', true],
    ['C:\\notes\\b.MARKDOWN', true],
    ['C:\\notes\\c.Md', true],
    ['C:\\notes\\d.txt', false],
    ['C:\\notes\\e.md.bak', false],
    ['C:\\notes\\mdfile', false],
    ['C:\\notes\\f.mdx', false],
  ];
  for (const [path, want] of cases) {
    assert.equal(isMarkdownName(path), want, path);
  }
});

test('filterMarkdownFiles keeps only Markdown files and drops directories and other files', () => {
  const items = [
    { path: 'C:\\notes\\a.md', isFile: true },
    { path: 'C:\\notes\\folder.md', isFile: false },
    { path: 'C:\\notes\\b.json', isFile: true },
    { path: 'C:\\notes\\c.MARKDOWN', isFile: true },
  ];
  assert.deepEqual(
    filterMarkdownFiles(items, (item) => item).map((item) => item.path),
    ['C:\\notes\\a.md', 'C:\\notes\\c.MARKDOWN'],
  );
});

test('filterMarkdownFiles keeps mapped WSL paths', () => {
  const items = [
    { path: '\\\\wsl.localhost\\Ubuntu-24.04\\home\\craig\\a.md', isFile: true },
    { path: '\\\\wsl.localhost\\Ubuntu-24.04\\home\\craig\\b.json', isFile: true },
  ];
  assert.deepEqual(
    filterMarkdownFiles(items, (item) => item).map((item) => item.path),
    ['\\\\wsl.localhost\\Ubuntu-24.04\\home\\craig\\a.md'],
  );
});

test('filterMarkdownFiles projects items through the view', () => {
  const items = [{ uri: { fsPath: 'C:\\x\\a.md' }, kind: 'file' as const }];
  assert.deepEqual(
    filterMarkdownFiles(items, (item) => ({ path: item.uri.fsPath, isFile: true })),
    items,
  );
});

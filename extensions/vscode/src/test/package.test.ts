import { strict as assert } from 'node:assert';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { test } from 'node:test';

interface Manifest {
  activationEvents: string[];
  engines: { vscode: string };
  devDependencies: Record<string, string>;
  dependencies?: Record<string, string>;
  contributes: {
    menus: Record<string, Array<{ command: string; when?: string }>>;
    configuration: { properties: Record<string, { scope?: string }> };
  };
}

const manifest: Manifest = JSON.parse(
  readFileSync(join(__dirname, '..', '..', 'package.json'), 'utf8'),
) as Manifest;

test('mdedit.executablePath is machine-scoped so a workspace cannot name the executable', () => {
  assert.equal(manifest.contributes.configuration.properties['mdedit.executablePath']?.scope, 'machine');
});

test('activation is implied by commands, with no wildcard', () => {
  assert.deepEqual(manifest.activationEvents, []);
});

test('every menu entry is gated on isWindows', () => {
  const entries = Object.values(manifest.contributes.menus).flat();
  assert.equal(entries.length, 5);
  for (const entry of entries) {
    assert.ok(entry.when?.startsWith('isWindows'), entry.command);
  }
});

test('the openExternal when clauses match exactly .md and .markdown by filename', () => {
  const wanted = 'isWindows && resourceFilename =~ /\\.(md|markdown)$/i';
  const whens = Object.values(manifest.contributes.menus)
    .flat()
    .filter((item) => item.command === 'mdedit.openExternal')
    .map((item) => item.when);
  assert.equal(whens.length, 4);
  for (const when of whens) {
    assert.equal(when, wanted);
  }
});

test('the extension has no runtime dependencies', () => {
  assert.deepEqual(manifest.dependencies ?? {}, {});
});

test('@types/vscode is pinned to the engines minimum', () => {
  assert.equal(manifest.engines.vscode, '^1.90.0');
  assert.equal(manifest.devDependencies['@types/vscode'], '1.90.0');
});

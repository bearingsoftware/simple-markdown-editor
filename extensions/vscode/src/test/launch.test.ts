import { strict as assert } from 'node:assert';
import { EventEmitter } from 'node:events';
import type { ChildProcess, SpawnOptions } from 'node:child_process';
import { test } from 'node:test';
import { editorArguments, type FileRef, isLocalFile, launchEditor, type Spawner } from '../launch';

const EXE = 'C:\\Users\\craig\\AppData\\Local\\Programs\\Simple Markdown Editor\\mdedit.exe';

interface Fake {
  readonly spawner: Spawner;
  readonly calls: Array<{ command: string; args: string[]; options: SpawnOptions }>;
  unrefCount: () => number;
}

function fakeSpawner(outcome: 'spawn' | 'error'): Fake {
  const calls: Fake['calls'] = [];
  let unrefs = 0;
  const spawner: Spawner = (command, args, options) => {
    calls.push({ command, args, options });
    const child = new EventEmitter() as EventEmitter & { unref: () => void };
    child.unref = () => {
      unrefs += 1;
    };
    setImmediate(() => {
      if (outcome === 'spawn') {
        child.emit('spawn');
      } else {
        child.emit('error', new Error('ENOENT'));
      }
    });
    return child as unknown as ChildProcess;
  };
  return { spawner, calls, unrefCount: () => unrefs };
}

test('isLocalFile accepts only file scheme with no authority', () => {
  const cases: Array<[FileRef, boolean]> = [
    [{ scheme: 'file', authority: '', fsPath: 'C:\\notes\\a.md' }, true],
    [{ scheme: 'untitled', authority: '', fsPath: 'Untitled-1' }, false],
    [{ scheme: 'vscode-remote', authority: 'wsl+Ubuntu', fsPath: '/home/a.md' }, false],
    [{ scheme: 'file', authority: 'server', fsPath: '\\share\\a.md' }, false],
    [{ scheme: 'file', authority: '', fsPath: '' }, false],
  ];
  for (const [ref, want] of cases) {
    assert.equal(isLocalFile(ref), want, JSON.stringify(ref));
  }
});

test('editorArguments passes awkward names through as one argument each', () => {
  const names = [
    'C:\\notes\\plain.md',
    'C:\\my notes\\with spaces.md',
    'C:\\work\\a & calc.md',
    'C:\\work\\100% done.md',
    'C:\\work\\say "hi".md',
    'C:\\work\\café日本語.md',
    "C:\\work\\it's.md",
  ];
  assert.deepEqual(editorArguments(names), names);
});

test('editorArguments rejects anything that is not an absolute drive path', () => {
  const bad = [[], ['-flag.md'], ['--register'], ['relative.md'], ['\\\\server\\share\\a.md'], ['\\\\?\\C:\\a.md'], ['']];
  for (const paths of bad) {
    assert.throws(() => editorArguments(paths), Error, JSON.stringify(paths));
  }
});

test('no argument the editor receives starts with a dash', () => {
  const paths = ['C:\\a.md', 'D:\\-b.md', 'E:\\--c.md'];
  for (const arg of editorArguments(paths)) {
    assert.equal(arg.startsWith('-'), false, arg);
  }
});

test('launchEditor spawns detached with an argument array and no shell', async () => {
  const fake = fakeSpawner('spawn');
  await launchEditor(EXE, ['C:\\work\\a & calc.md', 'C:\\work\\b.md'], fake.spawner);

  assert.equal(fake.calls.length, 1);
  const [call] = fake.calls;
  assert.equal(call?.command, EXE);
  assert.deepEqual(call?.args, ['C:\\work\\a & calc.md', 'C:\\work\\b.md']);
  assert.equal(call?.options.detached, true);
  assert.equal(call?.options.stdio, 'ignore');
  assert.equal(call?.options.windowsHide, false);
  assert.equal(call?.options.shell, undefined);
  assert.equal(fake.unrefCount(), 1);
});

test('launchEditor rejects when the executable cannot be started', async () => {
  const fake = fakeSpawner('error');
  await assert.rejects(launchEditor(EXE, ['C:\\a.md'], fake.spawner), /ENOENT/);
});

test('launchEditor spawns nothing for invalid input', async () => {
  const fake = fakeSpawner('spawn');
  await assert.rejects(launchEditor(EXE, ['--register'], fake.spawner));
  assert.equal(fake.calls.length, 0);
});

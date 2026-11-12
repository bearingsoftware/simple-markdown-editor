import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import { launchPath, remoteRecentPath, type UriParts } from '../wsl';

function remote(path: string, authority = 'wsl+Ubuntu-24.04'): UriParts {
  return { scheme: 'vscode-remote', authority, path, fsPath: path };
}

test('file scheme with no authority maps to its fsPath', () => {
  assert.equal(launchPath({ scheme: 'file', authority: '', path: '/c/a.md', fsPath: 'C:\\notes\\a.md' }), 'C:\\notes\\a.md');
});

test('file scheme that is untitled, remote-authored or empty is refused', () => {
  const cases: UriParts[] = [
    { scheme: 'untitled', authority: '', path: '/Untitled-1', fsPath: 'Untitled-1' },
    { scheme: 'file', authority: 'server', path: '/share/a.md', fsPath: '\\\\server\\share\\a.md' },
    { scheme: 'file', authority: '', path: '', fsPath: '' },
  ];
  for (const uri of cases) {
    assert.equal(launchPath(uri), undefined, JSON.stringify(uri));
  }
});

test('a WSL path maps to the wsl.localhost share', () => {
  const cases: Array<[string, string, string]> = [
    ['/home/craig/a.md', 'wsl+Ubuntu-24.04', '\\\\wsl.localhost\\Ubuntu-24.04\\home\\craig\\a.md'],
    ['/home/craig/my notes/a b.md', 'wsl+Ubuntu-24.04', '\\\\wsl.localhost\\Ubuntu-24.04\\home\\craig\\my notes\\a b.md'],
    ['/home/craig/100% done.md', 'wsl+Ubuntu-24.04', '\\\\wsl.localhost\\Ubuntu-24.04\\home\\craig\\100% done.md'],
    ['/home/craig/café日本語.md', 'wsl+Ubuntu-24.04', '\\\\wsl.localhost\\Ubuntu-24.04\\home\\craig\\café日本語.md'],
    ['/home/craig/a#b.md', 'wsl+Ubuntu-24.04', '\\\\wsl.localhost\\Ubuntu-24.04\\home\\craig\\a#b.md'],
    ['/home/craig/a.md', 'wsl+ubuntu-24.04', '\\\\wsl.localhost\\ubuntu-24.04\\home\\craig\\a.md'],
  ];
  for (const [path, authority, want] of cases) {
    assert.equal(launchPath(remote(path, authority)), want, path);
  }
});

test('a WSL path keeps a literal percent sign as itself', () => {
  const mapped = launchPath(remote('/home/craig/100% done.md'));
  assert.ok(mapped?.includes('100% done'), mapped);
});

test('a WSL distro name is refused unless it is a plain identifier of 1 to 64 characters', () => {
  const bad = ['wsl+a/b', 'wsl+..', 'wsl+', 'wsl+-x', `wsl+${'a'.repeat(65)}`];
  for (const authority of bad) {
    assert.equal(launchPath(remote('/home/a.md', authority)), undefined, authority);
  }
  assert.ok(launchPath(remote('/home/a.md', `wsl+${'a'.repeat(64)}`)));
});

test('a WSL path with a dot-dot or dot segment is refused', () => {
  for (const path of ['/home/../etc/a.md', '/home/./a.md', '/..']) {
    assert.equal(launchPath(remote(path)), undefined, path);
  }
});

test('a WSL path with NUL, backslash or no leading slash is refused', () => {
  for (const path of ['/home/a\0.md', '/home/a\\b.md', 'home/a.md']) {
    assert.equal(launchPath(remote(path)), undefined, JSON.stringify(path));
  }
});

test('a distro containing @ or a backslash, or ending in a dot, is refused', () => {
  for (const authority of ['wsl+a@b', 'wsl+a\\b', 'wsl+Ubuntu.']) {
    assert.equal(launchPath(remote('/home/a.md', authority)), undefined, authority);
  }
});

test('the wsl+ prefix is case-sensitive', () => {
  for (const authority of ['WSL+x', 'Wsl+x']) {
    assert.equal(launchPath(remote('/home/a.md', authority)), undefined, authority);
  }
});

test('a path segment ending in a dot or a space is refused', () => {
  for (const path of ['/home/a./b.md', '/home/a /b.md', '/home/craig/a.md.', '/home/craig/a.md ']) {
    assert.equal(launchPath(remote(path)), undefined, JSON.stringify(path));
  }
});

test('an empty, trailing-slash, colon or triple-dot segment is refused', () => {
  for (const path of ['/home//a.md', '/home/a.md/', '/', '/home/a:b.md', '/home/...', '/home/.../a.md']) {
    assert.equal(launchPath(remote(path)), undefined, JSON.stringify(path));
  }
});

test('remoteRecentPath maps a Windows drive path under /mnt with a lowercase drive', () => {
  const cases: Array<[string, string]> = [
    ['C:\\a\\b.md', '/mnt/c/a/b.md'],
    ['D:\\My Notes\\é.md', '/mnt/d/My Notes/é.md'],
    ['X:\\a.md', '/mnt/x/a.md'],
  ];
  for (const [windows, want] of cases) {
    assert.equal(remoteRecentPath(windows), want, windows);
  }
});

test('remoteRecentPath refuses UNC, drive-relative, and unsafe segments', () => {
  for (const windows of ['\\\\server\\share\\a.md', 'C:', 'C:a.md', 'C:\\a\\..\\b.md', 'C:\\a\\b.md.', 'C:\\']) {
    assert.equal(remoteRecentPath(windows), undefined, windows);
  }
});

test('other remote authorities and a bare vscode-remote are refused', () => {
  for (const authority of ['ssh-remote+host', 'dev-container+x', 'wsl', '']) {
    assert.equal(launchPath(remote('/home/a.md', authority)), undefined, authority);
  }
});

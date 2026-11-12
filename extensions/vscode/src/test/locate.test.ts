import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import { configuredExecutable, type LocateDeps, locateExecutable, parseInstallLocation } from '../locate';

const INSTALL_DIR = 'C:\\Users\\craig\\AppData\\Local\\Programs\\Simple Markdown Editor\\';
const REG_OUTPUT =
  '\r\nHKEY_CURRENT_USER\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\{6B1E2F84-3C5A-4D97-A0E8-7F2C9D41B365}_is1\r\n' +
  `    InstallLocation    REG_SZ    ${INSTALL_DIR}\r\n\r\n`;

const LOCAL = 'C:\\Users\\craig\\AppData\\Local';

type Over = Partial<LocateDeps> & { files?: string[] };

function deps(over: Over = {}): LocateDeps {
  const files = new Set(over.files ?? []);
  return {
    env: { LOCALAPPDATA: LOCAL },
    configured: undefined,
    readRegistry: async () => undefined,
    isFile: async (path) => files.has(path),
    ...over,
  };
}

test('parseInstallLocation reads the value from reg.exe output with CRLF endings', () => {
  assert.equal(parseInstallLocation(REG_OUTPUT), INSTALL_DIR);
});

test('parseInstallLocation returns undefined when the value is absent', () => {
  assert.equal(parseInstallLocation('ERROR: The system was unable to find the specified value.'), undefined);
});

test('locateExecutable applies user setting, registry, then LOCALAPPDATA in order', async () => {
  const configured = 'D:\\Portable\\mdedit.exe';
  const registryExe = `${INSTALL_DIR}mdedit.exe`;
  const fallbackExe = `${LOCAL}\\Programs\\Simple Markdown Editor\\mdedit.exe`;
  const all = [configured, registryExe, fallbackExe];

  const cases: Array<{ name: string; over: Over; want: string | undefined }> = [
    { name: 'configured wins', over: { configured, readRegistry: async () => REG_OUTPUT, files: all }, want: configured },
    { name: 'registry when no setting', over: { readRegistry: async () => REG_OUTPUT, files: all }, want: registryExe },
    { name: 'fallback when registry is absent', over: { files: all }, want: fallbackExe },
    { name: 'nothing found', over: {}, want: undefined },
    {
      name: 'configured path that is not a file falls through',
      over: { configured, readRegistry: async () => REG_OUTPUT, files: [registryExe] },
      want: registryExe,
    },
    {
      name: 'relative configured path is refused',
      over: { configured: 'mdedit.exe', files: ['mdedit.exe'] },
      want: undefined,
    },
    {
      name: 'configured path not named mdedit.exe is refused',
      over: { configured: 'C:\\tools\\calc.exe', files: ['C:\\tools\\calc.exe'] },
      want: undefined,
    },
    {
      name: 'UNC registry value is refused',
      over: {
        readRegistry: async () => '    InstallLocation    REG_SZ    \\\\server\\share\\app\\\r\n',
        files: ['\\\\server\\share\\app\\mdedit.exe'],
      },
      want: undefined,
    },
  ];

  for (const c of cases) {
    assert.equal(await locateExecutable(deps(c.over)), c.want, c.name);
  }
});

test('configuredExecutable reads only the global value, ignoring a workspace value', () => {
  const cases: Array<[{ globalValue?: unknown; workspaceValue?: unknown } | undefined, string | undefined]> = [
    [undefined, undefined],
    [{ workspaceValue: 'C:\\evil\\mdedit.exe' }, undefined],
    [{ globalValue: 'D:\\Portable\\mdedit.exe', workspaceValue: 'C:\\evil\\mdedit.exe' }, 'D:\\Portable\\mdedit.exe'],
    [{ globalValue: '' }, undefined],
    [{ globalValue: 42 }, undefined],
  ];
  for (const [inspected, want] of cases) {
    assert.equal(configuredExecutable(inspected), want, JSON.stringify(inspected));
  }
});

test('no LOCALAPPDATA and no registry means nothing is located', async () => {
  const found = await locateExecutable(
    deps({ env: {}, files: [`${LOCAL}\\Programs\\Simple Markdown Editor\\mdedit.exe`] }),
  );
  assert.equal(found, undefined);
});

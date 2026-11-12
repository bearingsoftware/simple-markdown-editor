import { win32 } from 'node:path';

export const EXE_NAME = 'mdedit.exe';

// Inno Setup's uninstall key for the per-user install (AppId in mdedit.iss).
export const REG_KEY =
  'HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\{6B1E2F84-3C5A-4D97-A0E8-7F2C9D41B365}_is1';

const ABSOLUTE_DRIVE = /^[A-Za-z]:\\/;

export interface LocateDeps {
  readonly env: Readonly<Record<string, string | undefined>>;
  readonly configured: string | undefined;
  readonly readRegistry: () => Promise<string | undefined>;
  readonly isFile: (path: string) => Promise<boolean>;
}

// Only the global (user or machine) value counts. A workspace value is never read here, so a
// repository cannot point the extension at an executable.
export function configuredExecutable(
  inspected: { readonly globalValue?: unknown } | undefined,
): string | undefined {
  const value = inspected?.globalValue;
  return typeof value === 'string' && value !== '' ? value : undefined;
}

export function parseInstallLocation(stdout: string): string | undefined {
  return /^\s+InstallLocation\s+REG_SZ\s+(.+?)\s*$/m.exec(stdout)?.[1];
}

async function usable(path: string, deps: LocateDeps): Promise<boolean> {
  return (
    ABSOLUTE_DRIVE.test(path) &&
    win32.basename(path).toLowerCase() === EXE_NAME &&
    (await deps.isFile(path))
  );
}

export async function locateExecutable(deps: LocateDeps): Promise<string | undefined> {
  if (deps.configured && (await usable(deps.configured, deps))) {
    return deps.configured;
  }

  const stdout = await deps.readRegistry();
  const installDir = stdout === undefined ? undefined : parseInstallLocation(stdout);
  if (installDir !== undefined) {
    const fromRegistry = win32.join(installDir, EXE_NAME);
    if (await usable(fromRegistry, deps)) {
      return fromRegistry;
    }
  }

  const localAppData = deps.env.LOCALAPPDATA;
  if (localAppData) {
    const fallback = win32.join(localAppData, 'Programs', 'Simple Markdown Editor', EXE_NAME);
    if (await usable(fallback, deps)) {
      return fallback;
    }
  }

  return undefined;
}

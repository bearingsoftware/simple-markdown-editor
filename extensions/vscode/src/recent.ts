import { win32 } from 'node:path';

export const MAX_ENTRIES = 10;
export const MAX_BYTES = 64 * 1024;

const LOCAL_DRIVE_PATH = /^[A-Za-z]:\\/;

export interface FileInfo {
  readonly size: number;
  readonly isFile: boolean;
}

export interface RecentDeps {
  readonly stat: (path: string) => Promise<FileInfo | undefined>;
  readonly read: (path: string) => Promise<string>;
}

export function recentListPath(env: Readonly<Record<string, string | undefined>>): string | undefined {
  return env.LOCALAPPDATA ? win32.join(env.LOCALAPPDATA, 'SimpleMarkdownEditor', 'recent.json') : undefined;
}

export function parseRecent(text: string): string[] {
  let parsed: unknown;
  try {
    parsed = JSON.parse(text);
  } catch {
    return [];
  }
  if (typeof parsed !== 'object' || parsed === null || !Array.isArray((parsed as { paths?: unknown }).paths)) {
    return [];
  }
  const entries = (parsed as { paths: unknown[] }).paths;
  return entries
    .filter(
      (entry): entry is string =>
        typeof entry === 'string' && LOCAL_DRIVE_PATH.test(entry) && !entry.includes('\0'),
    )
    .slice(0, MAX_ENTRIES);
}

export async function readRecent(file: string, deps: RecentDeps): Promise<string[]> {
  const info = await deps.stat(file);
  if (!info?.isFile || info.size > MAX_BYTES) {
    return [];
  }
  try {
    return parseRecent(await deps.read(file));
  } catch {
    return [];
  }
}

export async function keepExisting(paths: readonly string[], stat: RecentDeps['stat']): Promise<string[]> {
  const infos = await Promise.all(paths.map(stat));
  return paths.filter((_, i) => infos[i]?.isFile === true);
}

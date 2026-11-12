import { win32 } from 'node:path';

export const MAX_ENTRIES = 10;
export const MAX_BYTES = 64 * 1024;

const LOCAL_DRIVE_PATH = /^[A-Za-z]:\\/;

export interface FileInfo {
  readonly size: number;
  readonly isFile: boolean;
}

export interface RecentDeps {
  readonly readBounded: (path: string, limit: number) => Promise<Uint8Array | undefined>;
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

// One byte past the limit is read so that a file exactly at the limit is told apart from a larger one.
export async function readRecent(file: string, deps: RecentDeps): Promise<string[]> {
  let bytes: Uint8Array | undefined;
  try {
    bytes = await deps.readBounded(file, MAX_BYTES + 1);
  } catch {
    return [];
  }
  if (bytes === undefined || bytes.byteLength > MAX_BYTES) {
    return [];
  }
  return parseRecent(new TextDecoder().decode(bytes));
}

export async function keepExisting(
  paths: readonly string[],
  stat: (path: string) => Promise<FileInfo | undefined>,
): Promise<string[]> {
  const infos = await Promise.all(paths.map(stat));
  return paths.filter((_, i) => infos[i]?.isFile === true);
}

import { type ChildProcess, type SpawnOptions, spawn as nodeSpawn } from 'node:child_process';

export type Spawner = (command: string, args: string[], options: SpawnOptions) => ChildProcess;

// A drive path cannot begin with `-`, so this also keeps every argument clear of editor flags. The WSL
// prefix is only ever produced by `launchPath`, which has already refused `..` and backslashes.
const ABSOLUTE_DRIVE = /^[A-Za-z]:\\/;
const WSL_PREFIX = '\\\\wsl.localhost\\';

export function editorArguments(paths: readonly string[]): string[] {
  if (paths.length === 0) {
    throw new Error('no files to open');
  }
  for (const path of paths) {
    if (!ABSOLUTE_DRIVE.test(path) && !path.startsWith(WSL_PREFIX)) {
      throw new Error(`not an absolute local or WSL path: ${JSON.stringify(path)}`);
    }
  }
  return [...paths];
}

export async function launchEditor(
  exe: string,
  paths: readonly string[],
  spawn: Spawner = (command, args, options) => nodeSpawn(command, args, options),
): Promise<void> {
  const args = editorArguments(paths);
  const child = spawn(exe, args, { detached: true, stdio: 'ignore', windowsHide: false });
  await new Promise<void>((resolve, reject) => {
    child.once('error', reject);
    child.once('spawn', resolve);
  });
  child.unref();
}

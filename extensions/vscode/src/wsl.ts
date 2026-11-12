export interface UriParts {
  readonly scheme: string;
  readonly authority: string;
  readonly path: string;
  readonly fsPath: string;
}

const DISTRO = /^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$/;
const WSL_AUTHORITY = 'wsl+';

// Windows strips trailing dots and spaces, and `:` opens an alternate data stream, so a name carrying
// any of them can resolve to a different file than the one named. This also refuses some legitimate
// Linux names that contain `:`; that is accepted for safety.
export function isSafeSegment(segment: string): boolean {
  return segment !== '' && !/^\.+$/.test(segment) && !/[. ]$/.test(segment) && !segment.includes(':');
}

export function isSafePosixPath(path: string): boolean {
  return (
    path.startsWith('/') &&
    !path.includes('\0') &&
    !path.includes('\\') &&
    path.slice(1).split('/').every(isSafeSegment)
  );
}

// `path` is the decoded path that `Uri.path` gives, so a literal `%` or `#` in a file name arrives as itself.
export function launchPath(uri: UriParts): string | undefined {
  if (uri.scheme === 'file') {
    return uri.authority === '' && uri.fsPath !== '' ? uri.fsPath : undefined;
  }
  if (uri.scheme !== 'vscode-remote' || !uri.authority.startsWith(WSL_AUTHORITY)) {
    return undefined;
  }
  const distro = uri.authority.slice(WSL_AUTHORITY.length);
  if (!DISTRO.test(distro) || distro.endsWith('.') || !isSafePosixPath(uri.path)) {
    return undefined;
  }
  return `\\\\wsl.localhost\\${distro}${uri.path.replaceAll('/', '\\')}`;
}

// Maps a Windows path from the editor's recent list to where the same file sits under WSL's /mnt.
export function remoteRecentPath(windowsPath: string): string | undefined {
  const match = /^([A-Za-z]):\\(.*)$/.exec(windowsPath);
  if (match === null) {
    return undefined;
  }
  const [, drive = '', rest = ''] = match;
  const path = `/mnt/${drive.toLowerCase()}/${rest.replaceAll('\\', '/')}`;
  return isSafePosixPath(path) ? path : undefined;
}

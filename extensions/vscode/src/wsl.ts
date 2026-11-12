export interface UriParts {
  readonly scheme: string;
  readonly authority: string;
  readonly path: string;
  readonly fsPath: string;
}

const DISTRO = /^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$/;
const WSL_AUTHORITY = 'wsl+';

// `path` is the decoded path that `Uri.path` gives, so a literal `%` or `#` in a file name arrives as itself.
export function launchPath(uri: UriParts): string | undefined {
  if (uri.scheme === 'file') {
    return uri.authority === '' && uri.fsPath !== '' ? uri.fsPath : undefined;
  }
  if (uri.scheme !== 'vscode-remote' || !uri.authority.startsWith(WSL_AUTHORITY)) {
    return undefined;
  }
  const distro = uri.authority.slice(WSL_AUTHORITY.length);
  if (!DISTRO.test(distro) || !uri.path.startsWith('/') || uri.path.includes('\0') || uri.path.includes('\\')) {
    return undefined;
  }
  if (uri.path.split('/').some((segment) => segment === '.' || segment === '..')) {
    return undefined;
  }
  return `\\\\wsl.localhost\\${distro}${uri.path.replaceAll('/', '\\')}`;
}

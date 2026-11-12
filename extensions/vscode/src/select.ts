import { win32 } from 'node:path';

const MARKDOWN_NAME = /\.(md|markdown)$/i;

export interface Candidate {
  readonly path: string;
  readonly isFile: boolean;
}

export function isMarkdownName(path: string): boolean {
  return MARKDOWN_NAME.test(win32.basename(path));
}

export function filterMarkdownFiles<T>(items: readonly T[], view: (item: T) => Candidate): T[] {
  return items.filter((item) => {
    const candidate = view(item);
    return candidate.isFile && isMarkdownName(candidate.path);
  });
}

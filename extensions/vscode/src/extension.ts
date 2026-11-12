import { execFile } from 'node:child_process';
import { promises as fs } from 'node:fs';
import { join, win32 } from 'node:path';
import * as vscode from 'vscode';
import { isLocalFile, launchEditor } from './launch';
import { configuredExecutable, locateExecutable, type LocateDeps, REG_KEY } from './locate';
import { type FileInfo, keepExisting, readRecent, recentListPath } from './recent';
import { filterMarkdownFiles } from './select';

const INSTALL_COMMAND = 'winget install BearingSoftware.SimpleMarkdownEditor';
const RELEASES_URL = 'https://github.com/bearingsoftware/simple-markdown-editor/releases';

interface RecentItem extends vscode.QuickPickItem {
  readonly path: string;
}

async function stat(path: string): Promise<FileInfo | undefined> {
  try {
    const info = await fs.stat(path);
    return { size: info.size, isFile: info.isFile() };
  } catch {
    return undefined;
  }
}

async function isFileUri(uri: vscode.Uri): Promise<boolean> {
  try {
    return (await vscode.workspace.fs.stat(uri)).type === vscode.FileType.File;
  } catch {
    return false;
  }
}

// reg.exe writes in the console's OEM code page, which Node cannot decode without a library. A
// non-ASCII install path therefore comes back mangled, fails the isFile check, and the lookup falls
// through to the default location or the machine setting.
function readRegistry(): Promise<string | undefined> {
  return new Promise((resolve) => {
    execFile(
      join(process.env.SystemRoot ?? 'C:\\Windows', 'System32', 'reg.exe'),
      ['query', REG_KEY, '/v', 'InstallLocation'],
      { windowsHide: true },
      (error, stdout) => resolve(error ? undefined : stdout),
    );
  });
}

function locateDeps(): LocateDeps {
  return {
    env: process.env,
    configured: configuredExecutable(
      vscode.workspace.getConfiguration('mdedit').inspect<string>('executablePath'),
    ),
    readRegistry,
    isFile: async (path) => (await stat(path))?.isFile === true,
  };
}

function windowsOnly(): boolean {
  if (process.platform === 'win32') {
    return true;
  }
  void vscode.window.showInformationMessage('Simple Markdown Editor runs on Windows only.');
  return false;
}

async function openExternal(uri: vscode.Uri | undefined, uris: readonly vscode.Uri[] | undefined): Promise<void> {
  if (!windowsOnly()) {
    return;
  }
  const active = vscode.window.activeTextEditor?.document.uri;
  const targets = uris?.length ? uris : uri ? [uri] : active ? [active] : [];
  if (targets.length === 0) {
    return;
  }
  if (!targets.every(isLocalFile)) {
    await vscode.window.showInformationMessage('Simple Markdown Editor opens local files only.');
    return;
  }

  const files = await Promise.all(targets.map(isFileUri));
  const markdown = filterMarkdownFiles(
    targets.map((uri, i) => ({ uri, isFile: files[i] === true })),
    (item) => ({ path: item.uri.fsPath, isFile: item.isFile }),
  ).map((item) => item.uri);
  if (markdown.length === 0) {
    await vscode.window.showInformationMessage('Select Markdown files to open.');
    return;
  }

  const exe = await locateExecutable(locateDeps());
  if (exe === undefined) {
    const choice = await vscode.window.showErrorMessage(
      `Simple Markdown Editor is not installed. Install it with: ${INSTALL_COMMAND}`,
      'Open Releases Page',
    );
    if (choice !== undefined) {
      await vscode.env.openExternal(vscode.Uri.parse(RELEASES_URL));
    }
    return;
  }

  const wanted = new Set(markdown.map((target) => target.toString()));
  try {
    for (const doc of vscode.workspace.textDocuments) {
      if (doc.isDirty && wanted.has(doc.uri.toString()) && !(await doc.save())) {
        await vscode.window.showWarningMessage('Not opened: the document could not be saved.');
        return;
      }
    }
    await launchEditor(
      exe,
      markdown.map((target) => target.fsPath),
    );
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    await vscode.window.showErrorMessage(`Could not open in Simple Markdown Editor: ${message}`);
  }
}

async function openRecent(): Promise<void> {
  if (!windowsOnly()) {
    return;
  }
  const file = recentListPath(process.env);
  const listed = file === undefined ? [] : await readRecent(file, { stat, read: (path) => fs.readFile(path, 'utf8') });
  const existing = await keepExisting(listed, stat);
  if (existing.length === 0) {
    await vscode.window.showInformationMessage('Simple Markdown Editor has no recent files to offer.');
    return;
  }

  const items: RecentItem[] = existing.map((path) => ({
    label: win32.basename(path),
    description: win32.dirname(path),
    path,
  }));
  const picked = await vscode.window.showQuickPick(items, {
    placeHolder: 'Open a recent file from Simple Markdown Editor',
  });
  if (picked === undefined) {
    return;
  }
  const doc = await vscode.workspace.openTextDocument(vscode.Uri.file(picked.path));
  await vscode.window.showTextDocument(doc);
}

export function activate(context: vscode.ExtensionContext): void {
  context.subscriptions.push(
    vscode.commands.registerCommand('mdedit.openExternal', (uri?: vscode.Uri, uris?: vscode.Uri[]) =>
      openExternal(uri, uris),
    ),
    vscode.commands.registerCommand('mdedit.openRecent', () => openRecent()),
  );
}

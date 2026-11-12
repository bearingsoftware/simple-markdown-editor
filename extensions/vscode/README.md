# Simple Markdown Editor for VS Code

Opens Markdown files in [Simple Markdown Editor](https://github.com/bearingsoftware/simple-markdown-editor),
the native Windows Markdown editor.

- **Open in Simple Markdown Editor** on `.md` and `.markdown` files, from the Explorer context menu,
  the editor tab menu, the editor title bar, or the Command Palette. Several selected files open in
  one window launch.
- **Open Recent from Simple Markdown Editor** from the Command Palette lists the files the editor
  opened last, and opens the one you pick in VS Code.

Requires Windows and the editor installed:

```
winget install BearingSoftware.SimpleMarkdownEditor
```

The extension finds the editor through its registry entry. Registry output is decoded as UTF-8, so an
install path containing non-ASCII characters is not matched; the extension then uses the default
location under `%LOCALAPPDATA%`, or `mdedit.executablePath` if you set it.

Local files and files in a WSL window are supported. A file in a WSL window (`wsl+<distro>`) opens through
`\\wsl.localhost\<distro>\...`, so the extension installs and runs in the Windows UI side of a WSL window.
The editor does not restore WSL files after a restart or list them in its recent files, so they do not
appear in Open Recent. Untitled, other remote and virtual documents are refused with a message.
Unsaved changes are saved before the file opens in the editor; if the save fails, nothing opens.

If the editor is installed somewhere the extension cannot find it, set `mdedit.executablePath` in your
user settings to the full path of `mdedit.exe`. That is a user setting, so a workspace cannot set it.

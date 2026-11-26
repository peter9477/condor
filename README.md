# Condor

A lightweight, read-only viewer for folder trees and the documents in them, for Windows.

- Folder tree on the left (several root folders), viewing pane on the right.
- Renders **Markdown** (GitHub-flavoured), HTML pages, PDF, images, and plain text / source files.
- Shows *everything* on disk, including git-ignored files and nested repositories.
- Works with WSL paths (`\\wsl$\...`) and network shares; folders are read lazily in the background.
- Links between documents work, and so do file and folder paths written in code style
  (`src/main.rs`, `../README.md:42`, `src/ui/`) when they exist, relative to the document or
  to its root folder. The tree follows along. ← / →, Alt+Left/Right and the mouse's side
  buttons go back and forward.
- Each document's scroll position and text selection are kept, both while moving between
  documents and across restarts (PDFs keep their position only within the PDF viewer).
- `</>` (or Ctrl+U) switches Markdown, HTML and SVG files between the rendered view and
  highlighted source; each file remembers which view you last used.
- Files Condor can't show (spreadsheets, Word documents, other binaries) get **Open** and
  **Open with…** buttons; right-clicking any file in the tree offers the same. Programs and
  scripts ask before running.
- Search (box above the tree, Ctrl+Shift+F): words are matched against file names,
  folders and contents, including text inside Office files, PDFs and old .doc/.xls files.
  Documents matching every word come first, then matches in the name over the path over
  the contents; each result shows where the words occur. Results replace the tree until
  you clear the search (Esc). Uses the tree's filters; extracted text is cached in memory.
- Office documents (.docx/.doc, .xlsx/.xls, .vsdx/.vsd, .pptx, ...) are shown with the
  Windows preview handler their application installs (what Explorer's preview pane uses),
  or, if there is none, converted to PDF by LibreOffice (cached). ⚙ chooses between them
  for all files; the PDF / PREVIEW button in a document's bar switches one file.
- `file://` links in Markdown open in Condor, like relative links.
- Tags and stars: right-click a file or folder in the tree (or use ☆ and # in a document's
  bar) to star it or give it tags, shown as badges in the tree; `*` stars the item under
  the keyboard cursor. The chips above the tree filter it to items with any of the chosen
  tags, plus the folders leading to them. Tags are kept per environment, by path.
- File names that don't fit are shortened in the middle, keeping the end and the
  extension. Files Condor can't show are dimmed.
- Each document has a bar above it with its name and its own controls: back/forward, source
  view, AUTO reload, reload. ◫ splits the window into two panes (click again to switch
  between side by side and stacked; drag the divider to resize); the tree opens files in the
  focused pane. ⧉ opens a file in its own window, with no tree; right-clicking a file in the
  tree offers "Open in other pane" and "Open in new window". Splits and windows come back
  on restart.
- Remembers its state per environment: roots, expanded folders, open file, scroll position, window placement.

Built in Rust on [wry](https://github.com/tauri-apps/wry) and [tao](https://github.com/tauri-apps/tao)
(the window and webview crates underneath Tauri), using the WebView2 runtime that ships with Windows.
The exe is about 1 MB.

## Usage

```
condor.exe [--env NAME] [FOLDER]
```

| Option | Meaning |
|---|---|
| `FOLDER` | Add this folder as a root (if it isn't one already). |
| `--env NAME` | Use the named environment (default: `default`). |

State is saved in `%APPDATA%\Condor\environments\NAME.json`.

## Building

From WSL or Linux, cross-compiling with [cargo-xwin](https://github.com/rust-cross/cargo-xwin):

```sh
cargo xwin build --release --target x86_64-pc-windows-msvc
```

Or natively on Windows with the MSVC toolchain: `cargo build --release`.

`tools/deploy.sh` (from WSL) builds and installs `%USERPROFILE%\.local\bin\condor.exe`.
If Condor is running, the old exe is renamed to `condor.exe.old-<time>`, so open windows keep working and offer to restart into the new build.

## Security

Viewed documents are untrusted. They are shown in a separate webview that has no IPC
channel to the app. Rendered Markdown and text get a Content-Security-Policy that blocks scripts.

## Licence

MIT or Apache-2.0, at your option.

## Filters and file changes

- The filter button (⧩) opens a panel with these options:
  - Show hidden files (dot files and the Windows hidden attribute; off by default).
  - Hide git-ignored files.
  - Only viewable file types (folders and extensionless files always show).
  - Exclude patterns in .gitignore syntax, relative to each root.
- Open files are read in one go and never held open, so other programs can edit, move or delete them.
- When the open file changes on disk, a banner offers Reload, and you keep seeing the old version until you choose it.
  **AUTO** (or "Always auto-reload this file") makes that file reload by itself, keeping the scroll position.

## Settings

The ⚙ button opens settings shared by every Condor window (`%APPDATA%\Condor\settings.json`;
other open windows pick up changes within a couple of seconds):

- Document width: wide tables and code with text at reading width (default), full width, or
  reading width only.
- The file types that count as viewable for the "Only viewable file types" filter.

Markdown front matter (`---` YAML or `+++` TOML at the top of a file) is shown as a small table.

See [ROADMAP.md](ROADMAP.md) for planned features.

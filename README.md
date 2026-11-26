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
- Office documents (.docx/.doc, .xlsx/.xls, .vsdx/.vsd, .pptx, ...) can be shown three ways:
  Condor's built-in view (Word .docx files: rendered as pages, quickly, with the bundled
  [docx-preview](https://github.com/VolodymyrBaydalka/docxjs)); the Windows preview handler
  their application installs (what Explorer's preview pane uses); or converted to PDF by
  LibreOffice (cached). ⚙ sets the general choice. The selector in a document's bar
  (Built-in | PDF | Windows) switches that file type in all windows (a dot marks the type's
  choice; the settings panel lists these so they can be undone). Its lock keeps the current
  viewer for that one file; while locked, the selector changes only that file.
  The built-in view flows text onto pages as Word does (headings stay with what follows; long
  tables split between rows), numbers the pages, shows Windows metafile pictures (EMF/WMF,
  drawn to PNG by Windows), and has a contents menu (☰, top left) built from the document's
  table of contents, or its headings, which stays open beside the pages.
  Its finished pages are cached locally (`%LOCALAPPDATA%\Condor\docx`, the newest 60) by
  file, size and modification time, so returning to a document shows it at once without
  reading it again; a changed file is rendered afresh.
- `file://` links in Markdown open in Condor, like relative links.
- Zip files open like folders in the tree: files inside view as usual (Markdown with its
  relative links and images, Office documents, PDFs...). They're extracted on demand to a
  local cache, never beside the zip.
- Copy (Ctrl+C) puts a file itself on the clipboard, so it pastes into a chat or Explorer
  as an attachment or a copy; Copy path (Ctrl+Shift+C) copies its path. A pane's copy
  button copies the selection, or the whole document. Folders have Open in Terminal (a
  WSL shell for `\\wsl$` folders).
- The tree follows files being added and removed (open folders are checked every few seconds).
- Tags and stars: right-click a file or folder in the tree (or use ☆ and # in a document's
  bar) to star it or give it tags, shown as badges in the tree; `*` stars the item under
  the keyboard cursor. The chips above the tree filter it to items with any of the chosen
  tags, plus the folders leading to them. Tags are kept per environment, by path.
- Opening a file from the tree (click, Enter, a search result) puts the keyboard in the
  document, and Esc puts it back in the tree; Enter on the file already showing just goes
  back to it (no reload), so Enter and Esc switch between the two. In a document, Left/Right
  and Ctrl+PgUp/PgDn page back and forward: a page at a time in the built-in Word view,
  else a screenful (Left/Right not when the document scrolls sideways).
- Hiding: right-click → Hide (or "Hidden" in a document's # box) takes a file or folder out
  of the tree and out of search results, e.g. old revisions. The ⊘ N hidden chip above the
  tree shows them again, dimmed, until clicked again, so they can be unhidden.
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
condor.exe [FOLDER | FILE]
condor.exe --env NAME [FOLDER]
condor.exe --state FILE [FOLDER]
```

An *environment* is a saved state: roots, expanded folders, open files, splits, pop-outs,
tags, scroll positions, window placement.

| Start | Environment |
|---|---|
| `condor FOLDER` (e.g. `condor .`) | The folder's own, in `FOLDER\.condor\state.json`; created if needed, with the folder as its root. `.condor` holds a `.gitignore` of `*`, so git ignores it. |
| `condor FILE` | The nearest folder above the file that has an environment (else the file's folder), showing the file. |
| `condor` in a folder with `.condor` | That folder's. |
| `condor` anywhere else, or from the Start menu | A selector: known environments, most recently used first (Enter opens the top one), a new one in the current folder, or one in a folder you pick. |
| `--env NAME` | A named one in `%APPDATA%\Condor\environments\NAME.json`. |
| `--state FILE` | The state kept in FILE. |

With `--env` or `--state`, a `FOLDER` is added to that environment as a root.

Each environment opens in one window: starting it again brings that window to the front.
A folder that is slow to answer (a network share) shows a waiting window you can leave
for the list; a missing one returns to the list with a note. Documents slow to arrive
show "loading…" in their pane's bar while everything else stays usable.

Settings shared by all windows are in `%APPDATA%\Condor\settings.json` (or the file named
by `CONDOR_SETTINGS`; instances started that way don't change the recent list).

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

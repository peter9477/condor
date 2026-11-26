# Condor

A lightweight, read-only viewer for folder trees and the documents in them, for Windows.

- Folder tree on the left (several root folders), viewing pane on the right.
- Renders **Markdown** (GitHub-flavoured), HTML pages, PDF, images, and plain text / source files.
- Shows *everything* on disk, including git-ignored files and nested repositories.
- Works with WSL paths (`\\wsl$\...`) and network shares; folders are read lazily in the background.
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

See [ROADMAP.md](ROADMAP.md) for planned features.

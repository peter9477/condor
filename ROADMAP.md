# Condor roadmap

Ideas recorded for later; none of these are implemented yet.

## Viewing
- Tree auto-refresh (today a folder is re-listed when expanded or on F5 / ↻).
- Editable list of "viewable" extensions in the filter panel (stored as `viewable_exts`
  in the environment file; edit the JSON by hand for now).

- Split view: two or more document panes side by side or stacked, each with its own
  history and change banner; the tree opens into the focused pane.

## Tags and stars
- Tag files and folders with short words, shown as badges in the tree; a star is a
  special tag shown as ★. Stored by path in the environment state.
- Filter the tree to one or more tags: tagged items plus the folders leading to them,
  built from the stored paths, so no folder needs scanning.

## Environments and windows
- Environment selector UI (state is already one file per environment; `--env NAME` works).
- Multiple windows, each with its own environment or view.

## Annotations
- Notes attached to a file, or to a heading or text range in it, stored beside the
  environment state rather than in the documents.

## Pointing Claude at a section
- Select text or a heading and send a reference (path, heading anchor, quoted selection)
  to a running Claude session, chosen by session id from the session registry.
- Must keep the content webview isolated: Rust reads the selection with
  `evaluate_script_with_callback`; documents never get IPC.

## Integrated Claude chat
- A chat pane with the same context as the viewer (roots, open file, file names),
  able to answer questions about the documents.

## Security notes
- HTML documents may run scripts (without IPC). A page could still navigate to an external
  URL carrying data, which Condor hands to the default browser. Consider a per-root
  "scripts in HTML" switch, or a confirmation before opening external links from HTML.

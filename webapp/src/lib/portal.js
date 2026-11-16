// A `position: fixed` overlay rendered from inside a page is fixed to the
// nearest transformed ancestor, not to the viewport: the route host's page
// enter animates `transform` (M9, docs/agents/transitions.md), so EVERY fixed
// overlay a page renders has to leave the page's subtree.
//
// `use:toBody` is the one implementation of the nine copies that used to live
// in components (PlaylistMembersSheet, ChapterSheet, ShareSheet, FolderSheet,
// MoveToFolderSheet, InviteSheet, PlaylistPicker, AdminUsers, ExplorerStyles,
// FlowControls, FlashWarning). Files a lot already edits switch to it; no
// drive-by refactor (CLAUDE.md: one writer per file, a scoped change).
//
// It is an ACTION, so Svelte calls it with the node when the element mounts
// and with `destroy()` when it unmounts: append once, remove once.

/**
 * Move `node` to `<body>` and put it back to nothing on destroy.
 * @param {HTMLElement} node
 */
export function toBody(node) {
  document.body.appendChild(node);
  return {
    destroy() {
      node.parentNode?.removeChild(node);
    },
  };
}

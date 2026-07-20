// Restore the hierarchy tree's expansion (and selected-node highlight) after a
// full page reload. Background: the sidebar survives *soft* (Astro view-transition)
// navigations via `transition:persist`, but a *hard* reload — e.g. clicking a
// sensor-map pin or the "Gå til datatilegnelse" link, both of which full-load
// /measurements — rebuilds the sidebar from scratch, so `loadTree` re-fetches the
// company's children collapsed and the tree "folds up to the property level".
//
// We persist the set of expanded node-ids as the user toggles branches
// (hier-tree-set.js, wired from the tree's hyperscript via window.hierTree), then
// re-expand them whenever tree content swaps in. Re-expanding a node triggers its
// htmx `loadChildren`, whose afterSwap re-enters this restore → the expansion
// cascades down to the deepest saved branch. Presentation only (DOM + sessionStorage).
import { markExpanded, markCollapsed, expandedIds } from "./hier-tree-set.js";

/** CSS.escape with a small fallback (node ids contain '#'). */
function esc(s) {
  return window.CSS && CSS.escape ? CSS.escape(s) : s.replace(/["\\]/g, "\\$&");
}

/** Re-expand any saved-expanded nodes found directly under `container`. */
function restoreIn(container) {
  const ids = expandedIds(sessionStorage);
  if (ids.size === 0) return;
  for (const li of container.querySelectorAll("li[data-id]")) {
    if (!ids.has(li.getAttribute("data-id"))) continue;
    const wrapper = li.querySelector(":scope > .icon-wrapper");
    const list = li.querySelector(":scope > .nested-list");
    if (!wrapper || !list) continue;
    const toggle = wrapper.querySelector(".tree-toggle");
    if (toggle) toggle.classList.add("tree-toggle-expanded");
    if (list.style.display === "none") list.style.display = "";
    if (list.children.length === 0) {
      // Empty branch → load its children; the resulting afterSwap re-enters
      // restoreIn(list) and expands deeper saved nodes. Cascade terminates when
      // no saved node under the freshly-loaded list still needs loading.
      if (window.htmx) window.htmx.trigger(wrapper, "loadChildren");
    } else {
      restoreIn(list); // already loaded → recurse synchronously
    }
  }
}

/** Re-apply the orange "selected" highlight to the active node and scroll to it. */
function restoreSelection() {
  const sel = sessionStorage.getItem("selectedNodeId");
  if (!sel) return;
  const link = document.querySelector(`.node-name-link[data-node-id="${esc(sel)}"]`);
  if (!link) return;
  for (const l of document.querySelectorAll(".node-name-link.selected")) {
    l.classList.remove("selected");
  }
  link.classList.add("selected");
  link.scrollIntoView({ block: "nearest" });
}

/** True when an htmx swap targeted the tree root or one of its nested lists. */
function isTreeSwap(target) {
  return (
    target &&
    (target.id === "hier-tree-list" ||
      (target.classList && target.classList.contains("nested-list")))
  );
}

/** Idempotent: expose the persistence hooks and register the restore listener. */
export function initHierTreeState() {
  // Bound to real sessionStorage so the tree hyperscript can call these directly.
  window.hierTree = {
    markExpanded: (id) => markExpanded(id, sessionStorage),
    markCollapsed: (id) => markCollapsed(id, sessionStorage),
  };
  if (window.__hierTreeInit) return;
  window.__hierTreeInit = true;
  document.addEventListener("htmx:afterSwap", (e) => {
    if (!isTreeSwap(e.detail && e.detail.target)) return;
    restoreIn(e.detail.target);
    restoreSelection();
  });
}

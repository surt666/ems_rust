// Pure persistence of the hierarchy tree's expanded node-ids. DOM-free and
// storage-injectable so `node --test` can cover it. The DOM/restore wiring
// lives in hier-tree-state.js. See
// docs/superpowers/plans (sidebar tree-state fix) / the systematic-debugging note:
// a full page reload (e.g. the sensor-map pin → /measurements) rebuilds the
// sidebar collapsed; persisting which branches were open lets us re-expand them.

const KEY = "hierExpandedIds";

function read(storage) {
  try {
    return new Set(JSON.parse(storage.getItem(KEY) || "[]"));
  } catch {
    return new Set();
  }
}

function write(set, storage) {
  storage.setItem(KEY, JSON.stringify([...set]));
}

/** Record that the node `id` is expanded. No-op for empty ids. */
export function markExpanded(id, storage) {
  if (!id) return;
  const s = read(storage);
  s.add(id);
  write(s, storage);
}

/** Record that the node `id` is collapsed. No-op for empty ids. */
export function markCollapsed(id, storage) {
  if (!id) return;
  const s = read(storage);
  s.delete(id);
  write(s, storage);
}

/** The current set of expanded node-ids. */
export function expandedIds(storage) {
  return read(storage);
}

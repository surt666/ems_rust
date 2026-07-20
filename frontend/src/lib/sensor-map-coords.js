// Pure, DOM-free coordinate helpers for the sensor map. Unit-tested with
// `node --test`. No `import.meta.env` here — keep this importable by the runner.

/** Base point placeholder markers cluster around (Copenhagen). Swap for a real
 *  building coordinate when validated addresses land. */
export const BASE = { lat: 55.6761, lng: 12.5683 };

/** Max placeholder offset from BASE, in degrees (~150 m). */
const SPREAD = 0.0015;

/** Deterministic unsigned 32-bit FNV-1a hash of a string. */
export function hashString(s) {
  let h = 0x811c9dc5;
  const str = String(s);
  for (let i = 0; i < str.length; i++) {
    h ^= str.charCodeAt(i);
    h = Math.imul(h, 0x01000193);
  }
  return h >>> 0;
}

/** Map a 16-bit value to [-1, 1]. */
function unit16(n) {
  return (n / 0xffff) * 2 - 1;
}

/** Deterministic placeholder coordinate for a daqid, clustered around BASE. */
export function placeholderCoord(daqid) {
  const h = hashString(daqid);
  const latOff = unit16(h & 0xffff) * SPREAD;
  const lngOff = unit16((h >>> 16) & 0xffff) * SPREAD;
  return { lat: BASE.lat + latOff, lng: BASE.lng + lngOff };
}

/** Prefer an explicit finite {lat, lon}; otherwise fall back to the placeholder. */
export function resolveCoord(explicit, daqid) {
  const latRaw = explicit?.lat;
  const lonRaw = explicit?.lon;
  const hasLat = latRaw != null && String(latRaw).trim() !== "";
  const hasLon = lonRaw != null && String(lonRaw).trim() !== "";
  if (hasLat && hasLon) {
    const lat = Number(latRaw);
    const lon = Number(lonRaw);
    if (Number.isFinite(lat) && Number.isFinite(lon)) return { lat, lng: lon };
  }
  return placeholderCoord(daqid);
}

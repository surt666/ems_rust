/**
 * Attach the Cognito ID token to every call this app makes to its own APIs.
 *
 * Installed once from Layout.astro. Two interception points, because the app talks
 * to the backend two ways:
 *   - `fetch` — the React dashboard islands (get_aggregations, get_cost, …)
 *   - `htmx:configRequest` — every hx-get/hx-post fragment and htmx.ajax call
 *
 * Both must be covered, or enabling the API Gateway JWT authorizer 401s whichever
 * one was missed. Ship this BEFORE the authorizer: without an authorizer the header
 * is simply ignored, so there is no window where requests are unauthenticated
 * against a backend that requires them.
 *
 * The token is only added to same-origin API paths and to the aggregations API —
 * never to third-party hosts, which must not receive our credentials.
 */
import { fetchAuthSession } from "aws-amplify/auth";

/** Paths proxied by CloudFront to the hierarchy API (see infra/frontend/frontend.go). */
const HIERARCHY_PREFIXES = ["/command", "/query/", "/hierarchy/"];

function aggBase(): string {
  return (import.meta.env.PUBLIC_AGG_API_BASE_URL || "").replace(/\/$/, "");
}

/** Should this URL carry our token? */
export function isOwnApi(url: string): boolean {
  try {
    const u = new URL(url, window.location.origin);
    const base = aggBase();
    if (base && u.origin === new URL(base).origin) return true;
    if (u.origin !== window.location.origin) return false;
    return HIERARCHY_PREFIXES.some((p) => u.pathname === p || u.pathname.startsWith(p));
  } catch {
    return false;
  }
}

/**
 * Current ID token, or "" when signed out.
 *
 * Amplify caches and refreshes the session, so this is cheap to call per request;
 * it does NOT hit Cognito each time. Never throws — a failure here must degrade to
 * an unauthenticated request (and a clean 401) rather than break the caller.
 */
export async function idToken(): Promise<string> {
  try {
    const session = await fetchAuthSession();
    return session.tokens?.idToken?.toString() ?? "";
  } catch {
    return "";
  }
}

export function installAuthHeaders(): void {
  const w = window as any;
  if (w.__emsAuthHeadersInstalled) return;
  w.__emsAuthHeadersInstalled = true;

  // ── fetch ──
  const originalFetch = window.fetch.bind(window);
  window.fetch = async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
    if (!isOwnApi(url)) return originalFetch(input as any, init);

    const token = await idToken();
    if (!token) return originalFetch(input as any, init);

    const headers = new Headers(init?.headers || (input as Request).headers || undefined);
    if (!headers.has("Authorization")) headers.set("Authorization", `Bearer ${token}`);
    return originalFetch(input as any, { ...init, headers });
  };

  // ── htmx ──
  //
  // configRequest is synchronous, so the token cannot be awaited here. Amplify's
  // session is already resolved in memory after configureAmplify() + the first
  // fetch, so we keep a cached copy and refresh it in the background.
  let cached = "";
  const refresh = () => {
    idToken().then((t) => {
      cached = t;
    });
  };
  refresh();
  document.addEventListener("astro:page-load", refresh);

  document.body.addEventListener("htmx:configRequest", (evt: any) => {
    const path: string = evt.detail?.path ?? "";
    if (cached && isOwnApi(path)) {
      evt.detail.headers["Authorization"] = `Bearer ${cached}`;
    }
  });
}

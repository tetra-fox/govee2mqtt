// path prefix the app is mounted under. "/" when the daemon serves at origin
// root (standalone deploy, vite dev server); "/api/hassio_ingress/<token>/"
// when the addon runs behind home assistant's ingress proxy. computed from
// the current page url so the build doesn't need a baked-in base.
//
// without this, fetch("/api/foo") and new WebSocket("ws://host/ws") escape
// the ingress mount and hit the ha frontend instead of the addon.
export const BASE = location.pathname.replace(/[^/]*$/, "");

// join an absolute-style path ("/api/foo") with BASE. callers keep using
// "/api/..." literals so the routes are greppable and match the daemon's
// router; the leading slash gets stripped here before joining.
export function apiPath(path: string): string {
  return BASE + path.replace(/^\//, "");
}

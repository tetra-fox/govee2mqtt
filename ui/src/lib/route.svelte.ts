// hash-based router. each top-level view has its own url so the browser
// back/forward buttons work and deep links land on the right screen.
// hash routes avoid colliding with the daemon's /api and /ws routes.
//
// routes:
//   #/                   -> devices
//   #/devices            -> devices
//   #/devices/{id}       -> device detail
//   #/discovery          -> discovery
//   #/hass               -> hass
//   #/frames             -> frames
//   #/info               -> info

export type ViewKey = "devices" | "discovery" | "hass" | "frames" | "info";

const VIEWS: ViewKey[] = ["devices", "discovery", "hass", "frames", "info"];

type Parsed = { view: ViewKey; deviceId: string | null };

function parse(hash: string): Parsed {
  // strip leading '#' and any leading '/'
  const raw = hash.replace(/^#/, "").replace(/^\/+/, "");
  const parts = raw.split("/").filter(Boolean);
  if (parts.length === 0) return { view: "devices", deviceId: null };
  const first = parts[0] as ViewKey;
  if (!VIEWS.includes(first)) return { view: "devices", deviceId: null };
  if (first === "devices" && parts.length >= 2) {
    return { view: "devices", deviceId: decodeURIComponent(parts[1]) };
  }
  return { view: first, deviceId: null };
}

function format({ view, deviceId }: Parsed): string {
  if (view === "devices" && deviceId) return `#/devices/${encodeURIComponent(deviceId)}`;
  return `#/${view}`;
}

class Route {
  view = $state<ViewKey>("devices");
  deviceId = $state<string | null>(null);
  // increments whenever the parsed route changes. lets consumers $effect-react
  // to "any navigation happened" without diffing fields.
  tick = $state(0);

  #onHashChange = () => this.#refresh();

  start() {
    this.#refresh();
    window.addEventListener("hashchange", this.#onHashChange);
  }

  stop() {
    window.removeEventListener("hashchange", this.#onHashChange);
  }

  // navigate to a new route. pushes a history entry so browser back works.
  // calling go with the same parsed shape is a no-op so click handlers can
  // be wired without manual equality checks.
  go(next: Partial<Parsed>) {
    const target = format({
      view: next.view ?? this.view,
      deviceId: next.deviceId ?? null,
    });
    if (location.hash === target) return;
    location.hash = target;
  }

  // shortcut: open a device detail page on the devices view.
  openDevice(id: string) {
    this.go({ view: "devices", deviceId: id });
  }

  // shortcut: clear the detail panel and return to the device grid.
  backToGrid() {
    this.go({ view: "devices", deviceId: null });
  }

  #refresh() {
    const parsed = parse(location.hash);
    this.view = parsed.view;
    this.deviceId = parsed.deviceId;
    this.tick++;
  }
}

export const route = new Route();

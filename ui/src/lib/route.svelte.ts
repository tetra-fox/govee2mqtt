// hash-based router that writes via history.pushState so navigation never
// fires the `hashchange` event. assigning location.hash directly works fine
// standalone, but under home assistant ingress the parent frame listens for
// hashchange on the iframe's contentWindow and refreshes the whole iframe on
// each fire, which re-fetches every asset and rebuilds the websocket. z2m's
// hash router (react-router v7's HashRouter) avoids this for the same reason:
// pushState updates the url + history without firing hashchange. browser
// back/forward still works because popstate fires for those.
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

  #onPopState = () => this.#refresh();

  start() {
    this.#refresh();
    // popstate fires for browser back/forward (and pushState callers don't
    // need to react to themselves, they already updated state synchronously).
    window.addEventListener("popstate", this.#onPopState);
  }

  stop() {
    window.removeEventListener("popstate", this.#onPopState);
  }

  // navigate to a new route. pushes a history entry via pushState so the url
  // updates and browser back works, without firing hashchange.
  go(next: Partial<Parsed>) {
    const parsed: Parsed = {
      view: next.view ?? this.view,
      deviceId: next.deviceId ?? null,
    };
    if (parsed.view === this.view && parsed.deviceId === this.deviceId) return;
    const target = format(parsed);
    // build the absolute url preserving the current path + query so we only
    // change the hash. relative pushState would do the same, but spelling out
    // the path makes intent clear when reading the url in devtools.
    const url = `${location.pathname}${location.search}${target}`;
    history.pushState(null, "", url);
    this.view = parsed.view;
    this.deviceId = parsed.deviceId;
    this.tick++;
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

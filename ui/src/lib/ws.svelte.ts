import type { CommandLog, DeviceItem, Frame, StateEvent } from "./types";

// stable client-side ids so {#each} keys don't tear down rows on every update.
// the wire entries don't carry an id (command logs and frames are emitted as
// they happen) and start timestamps can collide within a millisecond. assigning
// a monotonic id on insert keeps the same row keyed across reverses and slices.
export type KeyedFrame = Frame & { _id: number };
export type KeyedCommandLog = CommandLog & { _id: number };

// connection states for the ui badge. 'lost' means the socket closed and we
// are about to retry; the snapshot we have is stale but still visible.
export type ConnStatus = "connecting" | "open" | "lost";

// mirrors COMMAND_HISTORY_CAP in src/service/state.rs. clients tail the same
// number of entries the daemon keeps so the live and refetched views match.
const COMMAND_HISTORY_CAP = 30;

// rolling buffer of recent frame events for the inspector. capped client-side
// since the daemon doesn't keep a frame history; refresh = clear and wait.
const FRAME_TAIL_CAP = 200;

class DeviceStore {
  devices = $state<DeviceItem[]>([]);
  status = $state<ConnStatus>("connecting");
  // monotonic counter so consumers can $derived against it if they want
  // to react to "any update happened" without diffing the list.
  tick = $state(0);
  // per-device command history. populated from /api/device/{id}/debug on
  // detail open and patched live from command_logged ws events; cap matches
  // the daemon's ring so the two views stay aligned.
  histories = $state<Record<string, KeyedCommandLog[]>>({});
  // rolling tail of recent frames for the inspector. starts empty on
  // connect; older entries drop off once FRAME_TAIL_CAP is reached.
  frames = $state<KeyedFrame[]>([]);

  #socket: WebSocket | null = null;
  #retryMs = 1000;
  #retryTimer: ReturnType<typeof setTimeout> | null = null;
  // monotonic counter for client-side ids assigned to frames and command
  // logs. one shared sequence is fine since the ids are only used as
  // {#each} keys, not exposed anywhere.
  #nextId = 1;

  connect() {
    if (this.#socket) return;
    const proto = location.protocol === "https:" ? "wss:" : "ws:";
    const url = `${proto}//${location.host}/ws`;

    this.status = "connecting";
    const ws = new WebSocket(url);
    this.#socket = ws;

    ws.addEventListener("open", () => {
      this.status = "open";
      this.#retryMs = 1000;
    });

    ws.addEventListener("message", (ev) => {
      try {
        const msg = JSON.parse(ev.data as string) as StateEvent;
        this.#apply(msg);
      } catch (e) {
        // a malformed frame from the daemon would be a rust-side bug; surface
        // it once instead of swallowing.
        console.error("ws frame parse failed", e);
      }
    });

    ws.addEventListener("close", () => {
      this.#socket = null;
      this.status = "lost";
      this.#scheduleRetry();
    });

    ws.addEventListener("error", () => {
      // close fires after error, so just let close handle the retry.
    });
  }

  disconnect() {
    if (this.#retryTimer) {
      clearTimeout(this.#retryTimer);
      this.#retryTimer = null;
    }
    if (this.#socket) {
      this.#socket.close();
      this.#socket = null;
    }
  }

  // server may send Snapshot at any time, including after a broadcast-channel
  // lag resync. always replace the local list on snapshot rather than
  // attempting to diff.
  #apply(ev: StateEvent) {
    if (ev.type === "snapshot") {
      this.devices = ev.devices;
    } else if (ev.type === "device_updated") {
      const i = this.devices.findIndex((d) => d.id === ev.device.id);
      if (i >= 0) {
        this.devices[i] = ev.device;
      } else {
        this.devices = [...this.devices, ev.device];
      }
    } else if (ev.type === "command_logged") {
      const prior = this.histories[ev.device_id] ?? [];
      const next = [...prior, { ...ev.entry, _id: this.#nextId++ }];
      if (next.length > COMMAND_HISTORY_CAP) next.splice(0, next.length - COMMAND_HISTORY_CAP);
      this.histories = { ...this.histories, [ev.device_id]: next };
    } else if (ev.type === "frame") {
      const frame: KeyedFrame = {
        _id: this.#nextId++,
        device_id: ev.device_id,
        direction: ev.direction,
        transport: ev.transport,
        ts: ev.ts,
        payload: ev.payload,
      };
      const next = [...this.frames, frame];
      if (next.length > FRAME_TAIL_CAP) next.splice(0, next.length - FRAME_TAIL_CAP);
      this.frames = next;
    }
    this.tick++;
  }

  clearFrames() {
    this.frames = [];
  }

  // backfill a device's history from a fetched bundle. call before opening
  // a detail panel so the user sees the daemon's full ring even if the ws
  // wasn't connected during earlier commands. assigns stable client ids so
  // the rows have a key independent of position when the list is reversed.
  setHistory(deviceId: string, history: CommandLog[]) {
    const keyed: KeyedCommandLog[] = history.map((entry) => ({ ...entry, _id: this.#nextId++ }));
    this.histories = { ...this.histories, [deviceId]: keyed };
  }

  #scheduleRetry() {
    if (this.#retryTimer) return;
    const delay = this.#retryMs;
    // cap at 15s. linear backoff is plenty; this isn't a hot reconnect loop.
    this.#retryMs = Math.min(this.#retryMs * 2, 15000);
    this.#retryTimer = setTimeout(() => {
      this.#retryTimer = null;
      this.connect();
    }, delay);
  }
}

export const store = new DeviceStore();

import { getRecent } from "./api";
import { apiPath } from "./base";
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

// rolling buffer of recent frame events for the inspector. matches the
// daemon's FRAME_HISTORY_CAP so a refresh-hydrated buffer is the same size as
// what gets retained going forward.
const FRAME_TAIL_CAP = 1000;

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
  // set by disconnect() and cleared by connect(). every WebSocket event
  // listener checks this before mutating state, because socket.close() fires
  // its close event asynchronously: without the guard, an in-flight close
  // event from a previous socket runs after we've already replaced
  // this.#socket with a new one and would null the live reference + flip
  // status back to "lost" + schedule a stray retry.
  #stopped = false;
  // monotonic counter for client-side ids assigned to frames and command
  // logs. one shared sequence is fine since the ids are only used as
  // {#each} keys, not exposed anywhere.
  #nextId = 1;

  connect() {
    if (this.#socket) return;

    this.#stopped = false;
    this.status = "connecting";

    // Hydrate frames + per-device histories from the daemon's retained rings
    // before the ws opens so a refresh during a quiet moment still shows the
    // last batch of traffic. Failure to fetch is non-fatal: the live stream
    // will fill in from this point on regardless.
    void this.#hydrateAndOpen();
  }

  async #hydrateAndOpen() {
    try {
      const bundle = await getRecent();
      if (this.#stopped) return;
      // assign stable client ids and trim to the cap. the daemon ships the
      // ring oldest-first so we keep that ordering.
      const keyedFrames: KeyedFrame[] = bundle.frames.map((f) => ({
        ...f,
        _id: this.#nextId++,
      }));
      if (keyedFrames.length > FRAME_TAIL_CAP) {
        keyedFrames.splice(0, keyedFrames.length - FRAME_TAIL_CAP);
      }
      this.frames = keyedFrames;
      const hydratedHistories: Record<string, KeyedCommandLog[]> = {};
      for (const [deviceId, entries] of Object.entries(bundle.histories)) {
        hydratedHistories[deviceId] = entries.map((entry) => ({
          ...entry,
          _id: this.#nextId++,
        }));
      }
      this.histories = hydratedHistories;
    } catch (e) {
      console.error("hydrate /api/recent failed", e);
    }
    if (this.#stopped) return;
    this.#openSocket();
  }

  #openSocket() {
    const proto = location.protocol === "https:" ? "wss:" : "ws:";
    const url = `${proto}//${location.host}${apiPath("/ws")}`;
    const ws = new WebSocket(url);
    this.#socket = ws;

    ws.addEventListener("open", () => {
      if (this.#socket !== ws) return;
      this.status = "open";
      this.#retryMs = 1000;
    });

    ws.addEventListener("message", (ev) => {
      if (this.#socket !== ws) return;
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
      // ignore the close event from a stale socket (one we already replaced
      // or one that fired after disconnect() asked us to stop).
      if (this.#stopped || this.#socket !== ws) return;
      this.#socket = null;
      this.status = "lost";
      this.#scheduleRetry();
    });

    ws.addEventListener("error", () => {
      // close fires after error, so just let close handle the retry.
    });
  }

  disconnect() {
    this.#stopped = true;
    // backoff doesn't survive a deliberate stop: a future reconnect should
    // start from a fresh 1s, not from wherever we'd retreated to.
    this.#retryMs = 1000;
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
        annotation: ev.annotation,
      };
      const next = [...this.frames, frame];
      if (next.length > FRAME_TAIL_CAP) next.splice(0, next.length - FRAME_TAIL_CAP);
      this.frames = next;
    }
    this.tick++;
  }

  clearFrames(deviceId?: string) {
    if (deviceId) {
      this.frames = this.frames.filter((f) => f.device_id !== deviceId);
    } else {
      this.frames = [];
    }
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

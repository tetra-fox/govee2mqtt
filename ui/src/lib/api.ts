import type {
  DebugInfo,
  DeviceDebug,
  DeviceEntity,
  DiscoveryItem,
  HassDebug,
  OneClick,
  RecentBundle,
} from "./types";

// the rest endpoints all GET. error responses come back as
// {"code":N,"msg":"..."} per src/service/http.rs.

async function call(path: string): Promise<unknown> {
  const r = await fetch(path);
  const body = await r.json().catch(() => null);
  if (!r.ok) {
    const msg = body && typeof body === "object" && "msg" in body ? String(body.msg) : r.statusText;
    throw new Error(`${path}: ${msg}`);
  }
  // Distinguish "endpoint succeeded with null body" (an empty 200 or a body
  // that didn't parse as JSON) from "endpoint returned null". The callers all
  // expect a concrete shape; returning null here would cast through and leave
  // the view rendering nothing with no diagnostic.
  if (body === null) {
    throw new Error(`${path}: empty or non-JSON response`);
  }
  return body;
}

async function postJson(path: string, body: unknown): Promise<unknown> {
  const r = await fetch(path, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
  const out = await r.json().catch(() => null);
  if (!r.ok) {
    const msg = out && typeof out === "object" && "msg" in out ? String(out.msg) : r.statusText;
    throw new Error(`${path}: ${msg}`);
  }
  return out;
}

export function powerOn(id: string) {
  return call(`/api/device/${encodeURIComponent(id)}/power/on`);
}

export function powerOff(id: string) {
  return call(`/api/device/${encodeURIComponent(id)}/power/off`);
}

export function outletPower(id: string, index: number, on: boolean) {
  const verb = on ? "on" : "off";
  return call(`/api/device/${encodeURIComponent(id)}/outlet/${index}/${verb}`);
}

export function setBrightness(id: string, level: number) {
  return call(`/api/device/${encodeURIComponent(id)}/brightness/${level}`);
}

export function setColorTemp(id: string, kelvin: number) {
  return call(`/api/device/${encodeURIComponent(id)}/colortemp/${kelvin}`);
}

export function setColor(id: string, color: string) {
  return call(`/api/device/${encodeURIComponent(id)}/color/${encodeURIComponent(color)}`);
}

export function setScene(id: string, scene: string) {
  return call(`/api/device/${encodeURIComponent(id)}/scene/${encodeURIComponent(scene)}`);
}

export function listScenes(id: string) {
  return call(`/api/device/${encodeURIComponent(id)}/scenes`);
}

export async function listOneClicks(): Promise<OneClick[]> {
  const body = await call("/api/oneclicks");
  return Array.isArray(body) ? (body as OneClick[]) : [];
}

export function activateOneClick(scene: string) {
  return call(`/api/oneclick/activate/${encodeURIComponent(scene)}`);
}

export async function listDiscovery(): Promise<DiscoveryItem[]> {
  const body = await call("/api/debug/discovery");
  return Array.isArray(body) ? (body as DiscoveryItem[]) : [];
}

export async function getHassDebug(): Promise<HassDebug> {
  return (await call("/api/debug/hass")) as HassDebug;
}

export async function getDeviceDebug(id: string): Promise<DeviceDebug> {
  return (await call(`/api/device/${encodeURIComponent(id)}/debug`)) as DeviceDebug;
}

export function forcePoll(id: string) {
  return call(`/api/device/${encodeURIComponent(id)}/poll`);
}

export async function getDebugInfo(): Promise<DebugInfo> {
  return (await call("/api/debug/info")) as DebugInfo;
}

export async function getRecent(): Promise<RecentBundle> {
  return (await call("/api/recent")) as RecentBundle;
}

export async function getDeviceEntities(id: string): Promise<DeviceEntity[]> {
  const body = await call(`/api/device/${encodeURIComponent(id)}/entities`);
  return Array.isArray(body) ? (body as DeviceEntity[]) : [];
}

export async function setCapability(
  id: string,
  instance: string,
  value: unknown,
): Promise<unknown> {
  return postJson(
    `/api/device/${encodeURIComponent(id)}/capability/${encodeURIComponent(instance)}`,
    { value },
  );
}

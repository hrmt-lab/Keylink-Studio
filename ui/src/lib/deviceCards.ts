import type {
  DeviceBatteryStatus,
  DeviceInfo,
  ProbeResult,
  StudioDeviceStatus,
} from "../types";

export interface HostLinkDeviceGroup {
  key: string;
  name: string;
  devices: DeviceInfo[];
  verified: boolean;
  errors: string[];
}

export interface DeviceCardModel {
  key: string;
  name: string;
  hostLink: HostLinkDeviceGroup | null;
  studio: StudioDeviceStatus | null;
  battery: DeviceBatteryStatus | null;
  supported: boolean;
}

export function groupProbeResults(results: ProbeResult[]): HostLinkDeviceGroup[] {
  const groups = new Map<string, HostLinkDeviceGroup>();
  for (const result of results) {
    const key = result.device.device_uid_hash ?? `path:${result.device.path}`;
    const existing = groups.get(key);
    if (existing) {
      existing.devices.push(result.device);
      existing.name = chooseGroupName(existing.name, deviceDisplayName(result.device));
      existing.verified ||= result.verified;
      if (result.error) existing.errors.push(result.error);
    } else {
      groups.set(key, {
        key,
        name: deviceDisplayName(result.device),
        devices: [result.device],
        verified: result.verified,
        errors: result.error ? [result.error] : [],
      });
    }
  }
  return sortHostLinkGroups(groups);
}

export function groupVerifiedHostLinkDevices(devices: DeviceInfo[]): HostLinkDeviceGroup[] {
  const groups = new Map<string, HostLinkDeviceGroup>();
  for (const device of devices) {
    const key = device.device_uid_hash ?? `path:${device.path}`;
    const existing = groups.get(key);
    if (existing) {
      existing.devices.push(device);
      existing.name = chooseGroupName(existing.name, deviceDisplayName(device));
    } else {
      groups.set(key, {
        key,
        name: deviceDisplayName(device),
        devices: [device],
        verified: true,
        errors: [],
      });
    }
  }
  return sortHostLinkGroups(groups);
}

export function buildDeviceCards(
  hostLinkGroups: HostLinkDeviceGroup[],
  studioDevices: StudioDeviceStatus[],
  batteries: DeviceBatteryStatus[]
): DeviceCardModel[] {
  const usedHostLinkKeys = new Set<string>();
  const cards: DeviceCardModel[] = [];

  for (const studio of [...studioDevices].sort(compareStudioDevices)) {
    const hostLink = findHostLinkGroupForStudio(hostLinkGroups, studio, usedHostLinkKeys);
    if (hostLink) usedHostLinkKeys.add(hostLink.key);
    const hostDevices = hostLink?.devices ?? [];
    cards.push({
      key: `studio:${studio.id}:${hostLink?.key ?? "solo"}`,
      name: chooseCardName(hostLink, studio),
      hostLink,
      studio,
      battery: findBatteryForCard(batteries, hostDevices, studio),
      supported: isSupportedDevice(hostLink, studio),
    });
  }

  for (const hostLink of hostLinkGroups) {
    if (usedHostLinkKeys.has(hostLink.key)) continue;
    cards.push({
      key: `host:${hostLink.key}`,
      name: hostLink.name,
      hostLink,
      studio: null,
      battery: findBatteryForCard(batteries, hostLink.devices, null),
      supported: isSupportedDevice(hostLink, null),
    });
  }

  return cards.sort(compareCards);
}

export function supportedDeviceCount(
  hostLinkDevices: DeviceInfo[],
  studioDevices: StudioDeviceStatus[],
  batteries: DeviceBatteryStatus[]
): number {
  return buildDeviceCards(
    groupVerifiedHostLinkDevices(hostLinkDevices),
    studioDevices,
    batteries
  ).filter((card) => card.supported).length;
}

export function uniqueConnectionTypes(devices: DeviceInfo[]): DeviceInfo["connection_type"][] {
  return [...new Set(devices.map((device) => device.connection_type))].sort(
    (a, b) => connectionTypeRank(a) - connectionTypeRank(b)
  );
}

export function knownHostLinkConnectionTypes(
  devices: DeviceInfo[]
): DeviceInfo["connection_type"][] {
  return uniqueConnectionTypes(devices).filter((type) => type === "usb" || type === "bluetooth");
}

export function hasKnownConnectionType(card: DeviceCardModel): boolean {
  if (card.hostLink && knownHostLinkConnectionTypes(card.hostLink.devices).length > 0) return true;
  return isKnownStudioConnectionType(card.studio);
}

export function isKnownStudioConnectionType(studio: StudioDeviceStatus | null): boolean {
  return studio?.connection_type === "usb_serial" || studio?.connection_type === "ble_studio";
}

export function connectionTypeRank(connectionType: DeviceInfo["connection_type"]): number {
  if (connectionType === "usb") return 0;
  if (connectionType === "bluetooth") return 1;
  return 2;
}

function sortHostLinkGroups(groups: Map<string, HostLinkDeviceGroup>): HostLinkDeviceGroup[] {
  return [...groups.values()]
    .map((group) => ({
      ...group,
      devices: [...group.devices].sort(compareDeviceTransport),
    }))
    .sort(compareHostLinkGroups);
}

function isSupportedDevice(
  hostLink: HostLinkDeviceGroup | null,
  studio: StudioDeviceStatus | null
): boolean {
  if (hostLink?.verified) return true;
  return studio?.rpc_status === "ok";
}

function compareCards(a: DeviceCardModel, b: DeviceCardModel): number {
  return (
    Number(b.hostLink?.verified ?? false) - Number(a.hostLink?.verified ?? false) ||
    Number(Boolean(b.studio)) - Number(Boolean(a.studio)) ||
    a.name.localeCompare(b.name, undefined, { sensitivity: "base", numeric: true }) ||
    a.key.localeCompare(b.key)
  );
}

function chooseCardName(hostLink: HostLinkDeviceGroup | null, studio: StudioDeviceStatus): string {
  if (hostLink && hostLink.name !== "Unknown Device") return hostLink.name;
  return studio.display_name || studio.product || studio.manufacturer || studio.port_name;
}

function findHostLinkGroupForStudio(
  groups: HostLinkDeviceGroup[],
  studioDevice: StudioDeviceStatus,
  usedKeys: Set<string>
): HostLinkDeviceGroup | null {
  const candidates = groups.filter((group) => !usedKeys.has(group.key));

  const uidMatch = candidates.find((group) =>
    group.devices.some((device) => uidStringsMatch(device.device_uid_hash, studioDevice.serial_number))
  );
  if (uidMatch) return uidMatch;

  const serialMatch = candidates.find((group) =>
    group.devices.some((device) => serialsMatch(device.serial_number, studioDevice.serial_number))
  );
  if (serialMatch) return serialMatch;

  const productManufacturerMatch = candidates.find((group) =>
    group.devices.some((device) =>
      normalizedName(device.product) !== null &&
      normalizedName(device.product) === normalizedName(studioDevice.product) &&
      normalizedName(device.manufacturer) !== null &&
      normalizedName(device.manufacturer) === normalizedName(studioDevice.manufacturer)
    )
  );
  if (productManufacturerMatch) return productManufacturerMatch;

  const names = studioDeviceNames(studioDevice);
  if (names.size === 0) return null;
  const productMatches = candidates.filter((group) =>
    group.devices.some((device) => {
      const product = normalizedName(device.product);
      return product !== null && names.has(product);
    })
  );
  return productMatches.length === 1 ? productMatches[0] : null;
}

function compareHostLinkGroups(a: HostLinkDeviceGroup, b: HostLinkDeviceGroup): number {
  return (
    Number(b.verified) - Number(a.verified) ||
    a.name.localeCompare(b.name, undefined, { sensitivity: "base", numeric: true }) ||
    a.key.localeCompare(b.key)
  );
}

function compareStudioDevices(a: StudioDeviceStatus, b: StudioDeviceStatus): number {
  return (
    Number(b.rpc_status === "ok") - Number(a.rpc_status === "ok") ||
    studioViewerRank(a) - studioViewerRank(b) ||
    a.display_name.localeCompare(b.display_name) ||
    a.port_name.localeCompare(b.port_name)
  );
}

function compareDeviceTransport(a: DeviceInfo, b: DeviceInfo): number {
  return connectionTypeRank(a.connection_type) - connectionTypeRank(b.connection_type) || a.path.localeCompare(b.path);
}

function studioViewerRank(device: StudioDeviceStatus): number {
  if (device.keymap_viewer_status === "available") return 0;
  if (device.keymap_viewer_status === "locked") return 1;
  if (device.keymap_viewer_status === "unsupported") return 2;
  return 3;
}

function deviceDisplayName(device: DeviceInfo): string {
  return device.product ?? device.manufacturer ?? device.serial_number ?? device.device_uid_hash ?? "Unknown Device";
}

function chooseGroupName(current: string, next: string): string {
  if (current === "Unknown Device") return next;
  return current;
}

function serialsMatch(a: string | null, b: string | null): boolean {
  if (!a || !b) return false;
  return a.trim().toLowerCase() === b.trim().toLowerCase();
}

function uidHex(value: string | null | undefined): string | null {
  const normalized = value?.trim().toLowerCase();
  if (!normalized) return null;
  const hex = normalized.startsWith("uid:") ? normalized.slice(4) : normalized;
  return /^[0-9a-f]{16}$/.test(hex) ? hex : null;
}

function uidStringsMatch(a: string | null | undefined, b: string | null | undefined): boolean {
  const uidA = uidHex(a);
  const uidB = uidHex(b);
  return uidA !== null && uidA === uidB;
}

function normalizedName(value: string | null | undefined): string | null {
  const normalized = value?.trim().toLowerCase();
  return normalized ? normalized : null;
}

function studioDeviceNames(device: StudioDeviceStatus): Set<string> {
  return new Set(
    [device.product, device.display_name]
      .map(normalizedName)
      .filter((value): value is string => value !== null)
  );
}

function findBatteryForCard(
  batteries: DeviceBatteryStatus[],
  hostDevices: DeviceInfo[],
  studio: StudioDeviceStatus | null
): DeviceBatteryStatus | null {
  for (const device of hostDevices) {
    const battery =
      batteries.find(
        (entry) =>
          (device.device_uid_hash !== null && entry.device_key === device.device_uid_hash) ||
          serialsMatch(entry.serial_number, device.serial_number) ||
          (entry.product !== null && entry.product === device.product)
      ) ?? null;
    if (battery) return battery;
  }
  if (studio) {
    return (
      batteries.find(
        (entry) =>
          uidStringsMatch(entry.device_key, studio.serial_number) ||
          serialsMatch(entry.serial_number, studio.serial_number) ||
          (entry.product !== null && studioDeviceNames(studio).has(normalizedName(entry.product) ?? ""))
      ) ?? null
    );
  }
  return null;
}

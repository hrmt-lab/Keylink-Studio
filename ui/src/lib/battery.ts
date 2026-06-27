import type { DeviceBatterySource } from "../types";

export interface DisplayBatterySource {
  source: number;
  label: string;
  level: number;
}

export function batterySourceLabel(source: number): string {
  return source === 0 ? "C" : `P${source}`;
}

export function displayBatterySources(sources: DeviceBatterySource[]): DisplayBatterySource[] {
  return sources.flatMap((source) => {
    if (source.level === null) return [];
    return [
      {
        source: source.source,
        label: batterySourceLabel(source.source),
        level: source.level,
      },
    ];
  });
}

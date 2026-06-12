export const PLACEMENT_ZONE_OPTIONS = [
  { id: "", label: "No placement" },
  { id: "left_half", label: "Left half" },
  { id: "right_half", label: "Right half" },
  { id: "maximize", label: "Maximize" },
] as const;

export type PlacementZoneId = (typeof PLACEMENT_ZONE_OPTIONS)[number]["id"];

export function placementZoneLabel(zone: string | undefined | null): string {
  if (!zone) return "—";
  return PLACEMENT_ZONE_OPTIONS.find((o) => o.id === zone)?.label ?? zone;
}

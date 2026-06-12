import { PLACEMENT_ZONE_OPTIONS } from "./placementZones";

type PlacementZoneSelectProps = {
  value: string | undefined;
  onChange: (zone: string | undefined) => void;
  allowEmpty?: boolean;
  ariaLabel?: string;
  className?: string;
};

export function PlacementZoneSelect({
  value,
  onChange,
  allowEmpty = true,
  ariaLabel = "Window placement",
  className = "editor-formula-input editor-tools-placement-select",
}: PlacementZoneSelectProps) {
  const options = allowEmpty
    ? PLACEMENT_ZONE_OPTIONS
    : PLACEMENT_ZONE_OPTIONS.filter((o) => o.id !== "");

  return (
    <select
      className={className}
      aria-label={ariaLabel}
      value={value ?? ""}
      onChange={(e) => {
        const next = e.target.value;
        onChange(next.length > 0 ? next : undefined);
      }}
    >
      {options.map((opt) => (
        <option key={opt.id || "none"} value={opt.id}>
          {opt.label}
        </option>
      ))}
    </select>
  );
}

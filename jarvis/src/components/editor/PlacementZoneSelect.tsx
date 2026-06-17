import { useId } from "react";
import { EditorSelect } from "../ui/EditorSelect";
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
  className = "editor-placement-select",
}: PlacementZoneSelectProps) {
  const id = useId();
  const options = allowEmpty
    ? PLACEMENT_ZONE_OPTIONS
    : PLACEMENT_ZONE_OPTIONS.filter((o) => o.id !== "");

  return (
    <EditorSelect
      id={id}
      value={value ?? ""}
      onChange={(next) => onChange(next.length > 0 ? next : undefined)}
      options={options.map((opt) => ({ value: opt.id, label: opt.label }))}
      ariaLabel={ariaLabel}
      className={`editor-select-wrap--formula${className ? ` ${className}` : ""}`}
    />
  );
}

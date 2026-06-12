import type { OpenTargetPreview, ResolvedTargetPreview } from "../../types";
import { placementZoneLabel } from "./placementZones";

export function formatResolvedTargetSummary(resolved: ResolvedTargetPreview): string {
  if ("app" in resolved) {
    return `App: ${resolved.app.display_name} (${resolved.app.exe_path})`;
  }
  if ("url" in resolved) {
    return `URL: ${resolved.url.url}`;
  }
  const top = resolved.ambiguous.app_candidates[0]?.display_name ?? "—";
  return `Ambiguous: ${resolved.ambiguous.query} (app: ${top}, url: ${resolved.ambiguous.url_candidate.url})`;
}

export function formatOpenTargetPreview(preview: OpenTargetPreview): {
  targetLine: string;
  placementLine: string;
  resolvedLine: string;
} {
  return {
    targetLine: preview.target || "(empty)",
    placementLine: placementZoneLabel(preview.placement ?? undefined),
    resolvedLine: formatResolvedTargetSummary(preview.resolved),
  };
}

export function defaultUserToolDraft(): {
  name: string;
  display_name: string;
  description: string;
  parameters: { name: string; param_type: string; description?: string; required: boolean; enum_values?: string[] }[];
  actions: { speak: { text: string } }[];
  enabled: boolean;
  builtin: boolean;
} {
  return {
    name: "",
    display_name: "",
    description: "",
    parameters: [
      {
        name: "text",
        param_type: "string",
        description: "Text to speak",
        required: true,
        enum_values: [],
      },
    ],
    actions: [{ speak: { text: "{{text}}" } }],
    enabled: true,
    builtin: false,
  };
}

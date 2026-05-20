import fs from "fs";
import path from "path";
import { describe, expect, it } from "vitest";

/**
 * Regression: MSVC include must be three levels above Hostx64/x64 (not two — that lands in bin/include).
 */
function msvcIncludeFromHostBin(hostBin) {
  return path.normalize(path.join(hostBin, "..", "..", "..", "include"));
}

describe("msvcIncludeFromHostBin", () => {
  it("finds stdbool.h when Build Tools MSVC layout is present", () => {
    const roots = [
      path.join(
        process.env["ProgramFiles(x86)"] ?? "C:\\Program Files (x86)",
        "Microsoft Visual Studio",
        "2022",
        "BuildTools",
        "VC",
        "Tools",
        "MSVC",
      ),
    ];
    let hostBin = null;
    for (const root of roots) {
      if (!fs.existsSync(root)) continue;
      const versions = fs
        .readdirSync(root, { withFileTypes: true })
        .filter((d) => d.isDirectory())
        .map((d) => d.name)
        .sort()
        .reverse();
      for (const ver of versions) {
        const candidate = path.join(root, ver, "bin", "Hostx64", "x64");
        if (fs.existsSync(path.join(candidate, "cl.exe"))) {
          hostBin = candidate;
          break;
        }
      }
      if (hostBin) break;
    }
    if (!hostBin) {
      return;
    }
    const inc = msvcIncludeFromHostBin(hostBin);
    expect(fs.existsSync(path.join(inc, "vcruntime.h"))).toBe(true);
    expect(fs.existsSync(path.join(inc, "stdbool.h"))).toBe(true);
  });
});

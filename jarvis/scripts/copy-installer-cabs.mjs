import { copyFile, readdir } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

if (process.platform === "win32") {
  const tauri = fileURLToPath(new URL("../src-tauri/", import.meta.url));
  const target = path.resolve(tauri, process.env.CARGO_TARGET_DIR || "target");
  const arch = process.arch === "ia32" ? "x86" : process.arch;
  const source = path.join(target, "release", "wix", arch);
  const destination = path.join(target, "release", "bundle", "msi");
  const cabinets = (await readdir(source)).filter((name) => name.endsWith(".cab"));
  if (!cabinets.length) throw new Error("MSI external cabinet payload is missing");
  for (const name of cabinets) {
    await copyFile(path.join(source, name), path.join(destination, name));
    console.log(`Installer payload: ${name}`);
  }
}

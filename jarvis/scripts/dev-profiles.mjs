/**
 * Dev profile resolver — platform (--mac/--win), scope (--audio), and isolated Cargo target dirs.
 */

import path from "path";

/** @typedef {"auto" | "mac" | "win"} PlatformFlag */
/** @typedef {"full" | "audio"} ScopeFlag */
/** @typedef {"metal" | "cuda" | "vulkan" | "cpu" | "none"} WhisperBackend */

/**
 * @typedef {object} DevProfile
 * @property {string} id
 * @property {PlatformFlag} platform
 * @property {ScopeFlag} scope
 * @property {string[]} cargoFeatures
 * @property {string[]} modelSets
 * @property {string | null} targetDir absolute path, or null for default src-tauri/target
 * @property {string} logLabel
 * @property {boolean} noDefaultFeatures
 * @property {boolean} usesDefaultTargetDir
 */

const HOST_FOR_PLATFORM = {
  mac: "darwin",
  win: "win32",
};

const PLATFORM_SLUG = {
  darwin: "mac",
  win32: "win",
  linux: "linux",
};

/**
 * @param {string[]} argv
 * @returns {{ extraArgs: string[], platformFlag: PlatformFlag, scopeFlag: ScopeFlag }}
 */
export function parseDevProfileFlags(argv) {
  /** @type {string[]} */
  const extraArgs = [];
  /** @type {PlatformFlag} */
  let platformFlag = "auto";
  /** @type {ScopeFlag} */
  let scopeFlag = "full";

  for (const arg of argv) {
    if (arg === "--mac") {
      if (platformFlag !== "auto" && platformFlag !== "mac") {
        throw new Error("dev-profiles: use only one of --mac or --win");
      }
      platformFlag = "mac";
      continue;
    }
    if (arg === "--win") {
      if (platformFlag !== "auto" && platformFlag !== "win") {
        throw new Error("dev-profiles: use only one of --mac or --win");
      }
      platformFlag = "win";
      continue;
    }
    if (arg === "--audio") {
      scopeFlag = "audio";
      continue;
    }
    extraArgs.push(arg);
  }

  return { extraArgs, platformFlag, scopeFlag };
}

/**
 * Validate explicit platform flag against host OS.
 * @param {PlatformFlag} platformFlag
 * @param {NodeJS.Platform} hostPlatform
 * @returns {{ ok: true } | { ok: false, message: string }}
 */
export function validatePlatformHost(platformFlag, hostPlatform) {
  if (platformFlag === "auto") {
    return { ok: true };
  }
  const expected = HOST_FOR_PLATFORM[platformFlag];
  if (hostPlatform === expected) {
    return { ok: true };
  }
  const hostLabel = PLATFORM_SLUG[hostPlatform] ?? hostPlatform;
  return {
    ok: false,
    message:
      `dev-profiles: --${platformFlag} requires host OS ${expected} (current: ${hostLabel} / ${hostPlatform}). ` +
      "Cross-compilation is not supported.",
  };
}

/**
 * @param {PlatformFlag} platformFlag
 * @param {NodeJS.Platform} hostPlatform
 * @returns {"mac" | "win" | "linux"}
 */
function resolvePlatformSlug(platformFlag, hostPlatform) {
  if (platformFlag === "mac") return "mac";
  if (platformFlag === "win") return "win";
  return PLATFORM_SLUG[hostPlatform] ?? "linux";
}

/**
 * @param {ScopeFlag} scope
 * @param {WhisperBackend} backend
 * @returns {string[]}
 */
function buildCargoFeatures(scope, backend) {
  /** @type {string[]} */
  const features = ["oww"];
  if (scope === "full") {
    features.push("llm-local");
  }
  if (backend !== "none" && backend !== "cpu") {
    features.push(`whisper-${backend}`);
    if (scope === "full") {
      features.push(`llm-${backend}`);
    }
  }
  return features;
}

/**
 * @param {ScopeFlag} scope
 * @returns {string[]}
 */
function modelSetsForScope(scope) {
  if (scope === "audio") {
    return ["wake", "whisper-tiny"];
  }
  return ["wake", "whisper-all", "llm"];
}

/**
 * @param {object} opts
 * @param {PlatformFlag} opts.platformFlag
 * @param {ScopeFlag} opts.scopeFlag
 * @param {NodeJS.Platform} opts.hostPlatform
 * @param {WhisperBackend} opts.backend
 * @param {string} opts.jarvisRoot
 * @returns {DevProfile}
 */
export function resolveDevProfile(opts) {
  const { platformFlag, scopeFlag, hostPlatform, backend, jarvisRoot } = opts;

  const validation = validatePlatformHost(platformFlag, hostPlatform);
  if (!validation.ok) {
    throw new Error(validation.message);
  }

  const platformSlug = resolvePlatformSlug(platformFlag, hostPlatform);
  const effectiveBackend = backend === "none" ? "cpu" : backend;
  const cargoFeatures = buildCargoFeatures(scopeFlag, backend);
  const modelSets = modelSetsForScope(scopeFlag);

  const isDefaultProfile =
    platformFlag === "auto" && scopeFlag === "full" && effectiveBackend !== "cpu";

  let targetDir = null;
  if (!isDefaultProfile) {
    const parts = [platformSlug];
    if (scopeFlag === "audio") {
      parts.push("audio", effectiveBackend);
    } else if (effectiveBackend === "cpu") {
      parts.push("cpu-full");
    } else {
      parts.push("full", effectiveBackend);
    }
    targetDir = path.join(jarvisRoot, ".cache", "cargo-target", parts.join("-"));
  }

  const id = [
    platformFlag === "auto" ? platformSlug : platformFlag,
    scopeFlag,
    effectiveBackend,
  ].join("-");

  const explicitPlatform =
    platformFlag !== "auto" ? ` (explicit --${platformFlag})` : "";
  const scopeNote = scopeFlag === "audio" ? ", --audio" : "";
  const logLabel = `${id}${explicitPlatform}${scopeNote}`;

  return {
    id,
    platform: platformFlag,
    scope: scopeFlag,
    cargoFeatures,
    modelSets,
    targetDir,
    logLabel,
    noDefaultFeatures: scopeFlag === "audio",
    usesDefaultTargetDir: targetDir === null,
  };
}

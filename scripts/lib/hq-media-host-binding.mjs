import { createHash } from "node:crypto";

export const HQ_HOST_BINDING_SCHEMA = "shellx-cut/hq-host-binding@2";
// This is the SHA-256 of the canonical projection of Release Studio's
// shellx-cut/windows-hq atlas surface. The private target itself stays in the
// private control plane and operator-provided binding, never in Cut source.
export const HQ_ATLAS_HQ_SURFACE_SHA256 = "bd90aa2fc2777a73323e661652b128fc7cd9ed6f05654a52285dacb0b4e92710";

function canonicalJsonValue(value) {
  if (value === null || typeof value === "string" || typeof value === "boolean") return JSON.stringify(value);
  if (typeof value === "number") {
    if (!Number.isFinite(value)) throw new Error("canonical JSON does not allow non-finite numbers");
    return JSON.stringify(value);
  }
  if (Array.isArray(value)) return `[${value.map(canonicalJsonValue).join(",")}]`;
  if (typeof value !== "object") throw new Error(`canonical JSON does not allow ${typeof value}`);
  return `{${Object.keys(value).sort().map((key) => `${JSON.stringify(key)}:${canonicalJsonValue(value[key])}`).join(",")}}`;
}

export function canonicalJsonSha256(value) {
  return createHash("sha256").update(canonicalJsonValue(value)).digest("hex");
}

function declaredTargetHostname(projection) {
  const target = String(projection?.surface?.target || "").trim();
  const separator = target.indexOf("\\");
  if (separator <= 0 || separator === target.length - 1) return null;
  const hostname = target.slice(0, separator);
  return /\s/.test(hostname) ? null : hostname;
}

function projectionFields(projection) {
  return {
    schema: typeof projection?.schema === "string" ? projection.schema : null,
    project: typeof projection?.project === "string" ? projection.project : null,
    surface: {
      id: typeof projection?.surface?.id === "string" ? projection.surface.id : null,
      platform: typeof projection?.surface?.platform === "string" ? projection.surface.platform : null,
      role: typeof projection?.surface?.role === "string" ? projection.surface.role : null,
      transport: typeof projection?.surface?.transport === "string" ? projection.surface.transport : null,
      shell: typeof projection?.surface?.shell === "string" ? projection.surface.shell : null,
      target: typeof projection?.surface?.target === "string" ? projection.surface.target : null,
      targetHostname: declaredTargetHostname(projection),
    },
  };
}

function hasRequiredProjectionFields(fields) {
  return Boolean(
    fields.schema
    && fields.project
    && fields.surface.id
    && fields.surface.platform
    && fields.surface.role
    && fields.surface.transport
    && fields.surface.shell
    && fields.surface.target
    && fields.surface.targetHostname,
  );
}

export function hqHostBindingValidation(binding, actualHostname, {
  pinnedProjectionSha256 = HQ_ATLAS_HQ_SURFACE_SHA256,
} = {}) {
  const projection = binding?.atlasProjection;
  const fields = projectionFields(projection);
  let observedProjectionSha256 = null;
  let canonicalError = null;
  if (projection && typeof projection === "object" && !Array.isArray(projection)) {
    try {
      observedProjectionSha256 = canonicalJsonSha256(projection);
    } catch (error) {
      canonicalError = error.message;
    }
  }
  const actual = String(actualHostname || "").trim();
  const expected = fields.surface.targetHostname;
  const checks = {
    bindingSchemaMatches: binding?.schema === HQ_HOST_BINDING_SCHEMA,
    atlasProjectionPresent: projection && typeof projection === "object" && !Array.isArray(projection),
    atlasProjectionFieldsPresent: hasRequiredProjectionFields(fields),
    canonicalProjectionAvailable: canonicalError === null && observedProjectionSha256 !== null,
    canonicalProjectionMatchesPinnedIdentity: observedProjectionSha256 === pinnedProjectionSha256,
    actualHostnameMatchesPinnedTarget: Boolean(expected) && expected.toLowerCase() === actual.toLowerCase(),
  };
  return {
    pinnedProjectionSha256,
    observedProjectionSha256,
    actualHostname: actual || null,
    projection: fields,
    canonicalError,
    checks,
  };
}

export function assertHqHost(hostPlatform = process.platform, binding, actualHostname, options) {
  if (hostPlatform !== "win32") {
    throw new Error("HQ 4K/8K qualification is reserved for the local Windows HQ workstation; GROK is reserved for routine native UI diagnostics");
  }
  const validation = hqHostBindingValidation(binding, actualHostname, options);
  const failed = Object.entries(validation.checks).filter(([, passed]) => !passed).map(([name]) => name);
  if (failed.length) {
    throw new Error(`HQ media qualification requires the pinned Release Studio windows-hq identity: ${failed.join(", ")}`);
  }
  return validation;
}

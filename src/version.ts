/**
 * Borgkit version — single source of truth.
 * Bump this before every release; the CLI, templates, and docs all read from here.
 */

export const VERSION = '0.1.0';

export interface VersionInfo {
  version:   string;
  major:     number;
  minor:     number;
  patch:     number;
  prerelease: string | null;
  buildDate: string;
}

export function parseVersion(v: string = VERSION): VersionInfo {
  const match = v.match(/^(\d+)\.(\d+)\.(\d+)(?:-(.+))?$/);
  if (!match) throw new Error(`Invalid version string: ${v}`);
  return {
    version:    v,
    major:      parseInt(match[1], 10),
    minor:      parseInt(match[2], 10),
    patch:      parseInt(match[3], 10),
    prerelease: match[4] ?? null,
    buildDate:  new Date().toISOString().split('T')[0],
  };
}

export function isCompatible(required: string, actual: string = VERSION): boolean {
  const r = parseVersion(required);
  const a = parseVersion(actual);
  // semver: same major, actual minor/patch >= required
  return a.major === r.major && (a.minor > r.minor || (a.minor === r.minor && a.patch >= r.patch));
}

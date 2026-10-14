// Reads and builds fixtures/aisdk/VERSIONS.json without losing the pinned
// integrity.
//
// Every entry has one shape, the one scripts/fetch-aisdk.sh verifies:
//
//   "<package>": { "version": "<x.y.z>", "integrity": "sha512-<base64>" }
//
// When the fixture recorder rewrites the file, the integrity is kept from the
// previous file if the version is unchanged. Otherwise it is taken from the
// recorder's package-lock.json if the lock pins that same version. If neither
// source has it, writing fails: an entry without integrity is not valid.

import { existsSync, readFileSync } from 'node:fs';

// The same checks as scripts/fetch-aisdk.sh: a plain npm package name, an
// exact version and a sha512 SRI.
const PACKAGE = /^(@[a-z0-9][a-z0-9._-]*\/)?[a-z0-9][a-z0-9._-]*$/;
const VERSION = /^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$/;
const SHA512 = /^sha512-[A-Za-z0-9+/]{86}==$/;

/** Read a JSON file, or return `fallback` when it does not exist. */
export function readJson(path, fallback) {
  return existsSync(path) ? JSON.parse(readFileSync(path, 'utf8')) : fallback;
}

/**
 * Throws unless `doc` is a non-empty VERSIONS.json document in which every
 * entry is `{version, integrity}` with a sha512 integrity; returns `doc`.
 */
export function checkVersions(doc, source = 'VERSIONS.json') {
  if (!doc || typeof doc !== 'object' || Array.isArray(doc)) {
    throw new Error(`${source}: not a JSON object`);
  }
  if (Object.keys(doc).length === 0) throw new Error(`${source}: no packages`);
  for (const [pkg, entry] of Object.entries(doc)) {
    const ok =
      PACKAGE.test(pkg) &&
      entry &&
      typeof entry === 'object' &&
      !Array.isArray(entry) &&
      typeof entry.version === 'string' &&
      VERSION.test(entry.version) &&
      typeof entry.integrity === 'string' &&
      SHA512.test(entry.integrity);
    if (!ok) {
      throw new Error(`${source}: ${pkg} is not {"version", "integrity": "sha512-..."}: ${JSON.stringify(entry)}`);
    }
  }
  return doc;
}

/** The VERSIONS.json at `path`, checked; `{}` when the file does not exist. */
export function readVersions(path) {
  return existsSync(path) ? checkVersions(JSON.parse(readFileSync(path, 'utf8')), path) : {};
}

/** `{pkg: {version, integrity}}` from a package-lock.json (lockfileVersion 2/3). */
export function lockIntegrity(lock) {
  const out = {};
  for (const [key, entry] of Object.entries(lock?.packages ?? {})) {
    if (!key.startsWith('node_modules/') || !entry?.integrity) continue;
    out[key.slice('node_modules/'.length)] = { version: entry.version, integrity: entry.integrity };
  }
  return out;
}

/**
 * The VERSIONS.json document for `installed` (`{pkg: version}`), keyed in
 * sorted order. Throws when no source has the integrity of a package, or when
 * the previous file and the lock disagree on it for the same version.
 *
 * @param {Record<string,string>} installed
 * @param {Record<string, {version: string, integrity: string}>} previous
 * @param {Record<string, {version: string, integrity: string}>} locked
 */
export function versionsDocument(installed, previous = {}, locked = {}) {
  const out = {};
  for (const pkg of Object.keys(installed).sort()) {
    const version = installed[pkg];
    const prev = previous[pkg]?.version === version ? previous[pkg].integrity : undefined;
    const lock = locked[pkg]?.version === version ? locked[pkg].integrity : undefined;
    if (prev && lock && prev !== lock) {
      throw new Error(`${pkg}@${version}: VERSIONS.json pins ${prev} but package-lock.json has ${lock}`);
    }
    const integrity = prev ?? lock;
    if (!integrity) {
      throw new Error(
        `${pkg}@${version}: no integrity in the previous VERSIONS.json or in package-lock.json ` +
          '(install the pinned version with npm first)',
      );
    }
    out[pkg] = { version, integrity };
  }
  return checkVersions(out);
}

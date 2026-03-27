#!/usr/bin/env node
/**
 * Called from semantic-release (@semantic-release/exec) so Cargo + npm
 * optional binary packages match the root package version.
 */
const fs = require("fs");
const path = require("path");

const root = path.join(__dirname, "..");
const version = process.argv[2];

if (!version || typeof version !== "string") {
  console.error("Usage: node scripts/sync-release-versions.cjs <semver>");
  process.exit(1);
}

function writeCargoToml() {
  const p = path.join(root, "Cargo.toml");
  let s = fs.readFileSync(p, "utf8");
  const next = s.replace(/^version\s*=\s*"[^"]*"/m, `version     = "${version}"`);
  if (next === s) {
    console.error("Could not patch Cargo.toml workspace version");
    process.exit(1);
  }
  fs.writeFileSync(p, next);
}

function writeCargoLock() {
  const p = path.join(root, "Cargo.lock");
  let s = fs.readFileSync(p, "utf8");
  const re = /(\[\[package\]\]\nname = "borgkit-cli"\nversion = )"[^"]*"/;
  if (!re.test(s)) {
    console.error('Could not find [[package]] name = "borgkit-cli" in Cargo.lock');
    process.exit(1);
  }
  fs.writeFileSync(p, s.replace(re, `$1"${version}"`));
}

function writeJson(rel, mutator) {
  const p = path.join(root, rel);
  const j = JSON.parse(fs.readFileSync(p, "utf8"));
  mutator(j);
  fs.writeFileSync(p, `${JSON.stringify(j, null, 2)}\n`);
}

writeJson("npm/borgkit-cli/package.json", (j) => {
  j.version = version;
  if (j.optionalDependencies) {
    for (const k of Object.keys(j.optionalDependencies)) {
      j.optionalDependencies[k] = version;
    }
  }
});

for (const pkg of [
  "npm/borgkit-cli-linux-x64/package.json",
  "npm/borgkit-cli-linux-arm64/package.json",
  "npm/borgkit-cli-darwin-x64/package.json",
  "npm/borgkit-cli-darwin-arm64/package.json",
  "npm/borgkit-cli-win32-x64/package.json",
]) {
  writeJson(pkg, (j) => {
    j.version = version;
  });
}

writeCargoToml();
writeCargoLock();

console.log(`sync-release-versions: set artifact versions to ${version}`);

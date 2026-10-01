import { execSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";

const versionArg = process.argv[2];
const versionPattern = /^\d+\.\d+\.\d+(?:-[0-9A-Za-z]+(?:[.-][0-9A-Za-z]+)*)?$/;

if (!versionArg) {
  console.error("Usage: npm run release -- <patch|minor|major|X.Y.Z|X.Y.Z-label>");
  process.exit(1);
}

const current = JSON.parse(readFileSync("src-tauri/tauri.conf.json", "utf8")).version;
const next = bump(current, versionArg);

if (!versionPattern.test(next)) {
  console.error(`Refusing version "${next}". Use X.Y.Z or X.Y.Z-label.`);
  process.exit(1);
}

if (next === current) {
  console.error(`Version is already ${current}.`);
  process.exit(1);
}

const dirty = execSync("git status --porcelain", { encoding: "utf8" });
if (dirty.trim()) {
  console.error("Working tree is dirty. Commit or stash changes before releasing.");
  process.exit(1);
}

const tag = `v${next}`;
if (tagExists(tag)) {
  console.error(`Tag ${tag} already exists.`);
  process.exit(1);
}

replaceDeclaredVersion("src-tauri/tauri.conf.json", /("version"\s*:\s*")([^"]+)(")/g, current, next, 1);
replaceDeclaredVersion("package.json", /("version"\s*:\s*")([^"]+)(")/g, current, next, 1);
replaceDeclaredVersion("package-lock.json", /("version"\s*:\s*")([^"]+)(")/g, current, next, 2);
replaceDeclaredVersion("src-tauri/Cargo.toml", /^(version\s*=\s*")([^"]+)(")/m, current, next, 1);
replaceDeclaredVersion("src-tauri/Cargo.lock", /(name = "lipi"\r?\nversion = ")([^"]+)(")/, current, next, 1);

execSync(
  "git add package.json package-lock.json src-tauri/tauri.conf.json src-tauri/Cargo.toml src-tauri/Cargo.lock",
  { stdio: "inherit" },
);
execSync(`git commit -m "Release ${tag}"`, { stdio: "inherit" });
execSync(`git tag ${tag}`, { stdio: "inherit" });
execSync("git push -u origin HEAD", { stdio: "inherit" });
execSync(`git push origin ${tag}`, { stdio: "inherit" });

console.log(`Pushed ${tag}. A hyphenated version is a dev prerelease. A plain version is stable.`);

function bump(currentVersion, kind) {
  if (versionPattern.test(kind)) return kind;
  const match = currentVersion.match(/^(\d+)\.(\d+)\.(\d+)/);
  if (!match || !["patch", "minor", "major"].includes(kind)) {
    console.error("Usage: npm run release -- <patch|minor|major|X.Y.Z|X.Y.Z-label>");
    process.exit(1);
  }
  let major = Number(match[1]);
  let minor = Number(match[2]);
  let patch = Number(match[3]);
  if (kind === "major") {
    major += 1;
    minor = 0;
    patch = 0;
  } else if (kind === "minor") {
    minor += 1;
    patch = 0;
  } else {
    patch += 1;
  }
  return `${major}.${minor}.${patch}`;
}

function tagExists(name) {
  try {
    execSync(`git rev-parse --verify --quiet refs/tags/${name}`, { stdio: "ignore" });
    return true;
  } catch {
    return false;
  }
}

function replaceDeclaredVersion(path, pattern, from, to, times) {
  const text = readFileSync(path, "utf8");
  let count = 0;
  const updated = text.replace(pattern, (match, prefix, value, suffix) => {
    if (value !== from || count >= times) return match;
    count += 1;
    return `${prefix}${to}${suffix}`;
  });
  if (count !== times) {
    console.error(`Expected ${times} version update(s) in ${path}, found ${count}.`);
    process.exit(1);
  }
  writeFileSync(path, updated);
}

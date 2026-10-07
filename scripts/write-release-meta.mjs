import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const [releasesPath, outDir, tag] = process.argv.slice(2);

if (!releasesPath || !outDir || !tag) {
  console.error("Usage: node scripts/write-release-meta.mjs <releases.json> <out-dir> <tag>");
  process.exit(1);
}

const releases = JSON.parse(readFileSync(releasesPath, "utf8"));
if (!Array.isArray(releases)) {
  console.error("Expected a GitHub releases array.");
  process.exit(1);
}

const published = releases.filter((release) => !release.draft);
const stable = published.filter((release) => !release.prerelease);
const current = published.find((release) => release.tag_name === tag);

if (!current) {
  console.error(`Published release ${tag} was not found.`);
  process.exit(1);
}
if (current.prerelease) {
  console.error(`${tag} is a prerelease. The site manifest is only written for stable releases.`);
  process.exit(1);
}

const changelog = (stable.some((release) => release.tag_name === tag) ? stable : [current, ...stable])
  .slice(0, 20)
  .map(toManifest);

mkdirSync(outDir, { recursive: true });
writeFileSync(join(outDir, "latest.json"), `${JSON.stringify(toManifest(current), null, 2)}\n`);
writeFileSync(join(outDir, "changelog.json"), `${JSON.stringify(changelog, null, 2)}\n`);

function toManifest(release) {
  const assets = release.assets ?? [];
  return {
    version: String(release.tag_name).replace(/^v/, ""),
    tag: release.tag_name,
    prerelease: false,
    published_at: release.published_at ?? null,
    notes: release.body ?? "",
    downloads: {
      windows_setup: pick(assets, ["Lipi-windows-setup.exe"], ["-setup.exe"]),
      windows_msi: pick(assets, ["Lipi-windows.msi"], [".msi"]),
      linux_deb: pick(assets, ["Lipi-linux.deb"], [".deb"]),
      linux_appimage: pick(assets, ["Lipi-linux.AppImage"], [".AppImage"]),
      macos_dmg: pick(assets, ["Lipi-macos.dmg"], [".dmg"]),
    },
  };
}

function pick(assets, names, suffixes) {
  for (const name of names) {
    const found = assets.find((asset) => asset.name === name);
    if (found?.browser_download_url) return found.browser_download_url;
  }
  for (const suffix of suffixes) {
    const found = assets.find((asset) => asset.name.endsWith(suffix) && !asset.name.endsWith(".sig"));
    if (found?.browser_download_url) return found.browser_download_url;
  }
  return null;
}

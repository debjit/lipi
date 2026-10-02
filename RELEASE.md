# Releasing Lipi

GitHub Releases is where people download Lipi. The site at [lipi.debjit.in](https://lipi.debjit.in) lives in another repository and reads a manifest this repo publishes after a stable release.

The version in [`src-tauri/tauri.conf.json`](src-tauri/tauri.conf.json) is the version CI publishes. It must match [`package.json`](package.json), [`package-lock.json`](package-lock.json), [`src-tauri/Cargo.toml`](src-tauri/Cargo.toml), and the `lipi` entry in `src-tauri/Cargo.lock`.

## Cut a release

Start from a clean working tree. The script refuses to run if there are uncommitted changes or if the tag already exists.

```bash
npm run release -- patch
npm run release -- minor
npm run release -- major
npm run release -- 0.2.0
npm run release -- 0.1.5-3
```

`patch`, `minor`, and `major` always produce a stable version with no hyphen. From `0.1.5-2`, `patch` becomes `0.1.6`. Pass an explicit version when you want the next dev build or a specific number.

The script bumps the five version files, commits `Release vX.Y.Z`, tags `vX.Y.Z`, and pushes the branch and the tag. Pushing the tag starts [`.github/workflows/release.yml`](.github/workflows/release.yml).

## Dev and stable

A version with a hyphen is a dev prerelease. `0.1.5-2` is dev. `0.2.0` is stable.

| | Dev (`0.1.5-3`) | Stable (`0.2.0`) |
| :--- | :--- | :--- |
| GitHub Release | Prerelease titled `Lipi v0.1.5-3 (dev)` | Latest release titled `Lipi v0.2.0` |
| Installers | Versioned Tauri filenames only | Versioned files, plus stable names below |
| `release-meta` | Unchanged | `latest.json` and `changelog.json` updated |
| Site webhook | Not called | POST of `latest.json` |
| In-app notice | Does not notify | Notifies installed copies of an older version |

CI builds Windows and Linux, uploads the installers to a draft, and publishes that draft only after both builds succeed. If either build fails, the draft stays unpublished.

## Stable download names

After a stable release, these files sit next to the versioned Tauri installers:

| Platform | Stable filename |
| :--- | :--- |
| Windows setup | `Lipi-windows-setup.exe` |
| Windows MSI | `Lipi-windows.msi` |
| Linux deb | `Lipi-linux.deb` |
| Linux AppImage | `Lipi-linux.AppImage` |

The README links to them through `/releases/latest/download/`. Those links work only after the first stable release. GitHub's "latest" URL ignores prereleases, so a dev tag never replaces them.

## Manifest for the website

A stable publish writes two files and pushes them to the `release-meta` branch:

- https://raw.githubusercontent.com/debjit/lipi/release-meta/latest.json
- https://raw.githubusercontent.com/debjit/lipi/release-meta/changelog.json

`latest.json` is the release that was just published. `changelog.json` is the latest 20 stable releases, newest first. Each entry looks like this:

```json
{
  "version": "0.2.0",
  "tag": "v0.2.0",
  "prerelease": false,
  "published_at": "2026-10-02T00:00:00Z",
  "notes": "Stable Lipi release.",
  "downloads": {
    "windows_setup": "https://github.com/debjit/lipi/releases/download/v0.2.0/Lipi-windows-setup.exe",
    "windows_msi": "https://github.com/debjit/lipi/releases/download/v0.2.0/Lipi-windows.msi",
    "linux_deb": "https://github.com/debjit/lipi/releases/download/v0.2.0/Lipi-linux.deb",
    "linux_appimage": "https://github.com/debjit/lipi/releases/download/v0.2.0/Lipi-linux.AppImage"
  }
}
```

Download URLs come from the assets GitHub actually uploaded. A missing installer is `null`.

## Site webhook

After `release-meta` is updated, CI POSTs `latest.json` to the GitHub Actions secret `LIPI_SITE_DEPLOY_HOOK`. Point that secret at the site host's deploy hook. The other site can use the JSON body for its download buttons and then redeploy.

`GITHUB_TOKEN` cannot start a workflow in another repository, so the hook URL is the credential. Dev releases do not call it. If the secret is empty, CI logs that and the release still publishes.

## Update notice in the app

Lipi does not install updates itself. On startup it reads `https://api.github.com/repos/debjit/lipi/releases/latest`, which ignores prereleases. If that version is newer than the running app, the navbar and the settings footer show an **Update** notice. Opening it downloads the installer for this operating system: the Windows setup, or the Linux deb. If GitHub cannot be reached, nothing is shown.

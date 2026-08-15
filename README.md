# Nexus Manager

Desktop mod manager for [Nexus Mods](https://www.nexusmods.com/) on **Linux** and **Windows**.

**MVP+ features**

- Detect installed games (Linux: Steam / Heroic / Lutris / Bottles via `lib_game_detector`; Windows: Steam registry + libraries, optional Heroic)
- Manage supported titles: **Stardew Valley**, **Baldur's Gate 3**, **Cyberpunk 2077**, **Days Gone**, **S.T.A.L.K.E.R. 2: Heart of Chornobyl**, **Warhammer 40,000: Darktide**, **Dark Souls** (PTDE), **Dark Souls Remastered**, **Dark Souls 2**, **Dark Souls 3**, **Elden Ring**, **Resident Evil 7**, **Village**, **Requiem**, **2/3/4 Remake**
- Days Gone: `.pak` mods deploy to `BendGame/Saved/Paks` (Windows `%LOCALAPPDATA%` or Steam Proton compatdata). First deploy renames `startuppackages.pak` → `startuppackages_modsenabled.pak` when needed so custom paks load.
- Darktide: mods deploy under `<game>/mods/`; enabled names are written to `mods/mod_load_order.txt`. Deploy runs `tools/dtkit-patch --patch bundle` (idempotent enable; not `--toggle`). Install the Darktide Mod Loader first so the patcher binary is present.
- RE Engine Resident Evil titles deploy `natives/`, `reframework/`, and `pak_mods/` into the game install (plus REF injectors like `dinput8.dll`). Install [REFramework](https://github.com/praydog/reframework) and enable **Loose File Loader** for loose-file mods; this app does not perform Fluffy-style PAK invalidation.
- FromSoftware titles deploy under `<Game>/mods/<ModName>/` (resolves the Steam `Game/` subfolder when present). Launch with an external injector such as [me3](https://github.com/garyttierney/me3), Mod Engine 2, Elden Mod Loader, or legacy Mod Engine — this app does not install or launch those tools.
- Search mods & Collections on Nexus (GraphQL v2 + REST v1)
- **Premium:** one-click API downloads and Collection batch install
- **Free:** visible **Download Assist** WebView on the Nexus file page (persistent website session in-app), optional **autoclick** (Nexus Fast Download–style) on Mod Manager / Slow Download when the button is enabled, keyed `nxm://` staging, local archive import, and **Collection install** via a sequential Download Assist queue
- Hardlink/symlink deploy with copy fallback (basic load order)
- Handle `nxm://` deep links from the Nexus website

## Requirements

- Linux or Windows 10+
- Rust toolchain + Node.js 20+
- **Linux:** system packages typically needed for Tauri: `webkit2gtk`, `libayatana-appindicator`, etc. (see [Tauri Linux prerequisites](https://v2.tauri.app/start/prerequisites/))
- **Windows:** [WebView2](https://developer.microsoft.com/microsoft-edge/webview2/) (installer can bootstrap it); optional [Developer Mode](https://learn.microsoft.com/windows/apps/get-started/enable-your-device-for-development) for symlink deploy instead of file copies
- A Nexus Mods account and **personal API key**
- **Nexus Premium** for one-click `download_link.json` without a site-issued key (and API Collection batch install)

## Setup

```bash
npm install
npm run tauri dev
```

Production build (host OS only):

```bash
npm run tauri build
```

Release builds for **both** Linux and Windows run in parallel via CI (`npm run build:release` documents this; see `.github/workflows/build.yml`).

### API key

1. Open [Nexus API Access](https://www.nexusmods.com/users/myaccount?tab=api)
2. Create / copy a personal API key
3. Paste it in **Setup** inside the app

The key is stored in the OS keyring when available (Linux Secret Service / Windows Credential Manager), otherwise in the app config directory as `nexus_api_key` (mode `0600` on Unix).

### Free-account downloads

1. Browse a mod file → **Download Assist**
2. A visible Nexus window opens. The **API key does not log you into the website** — the first time (or after clearing the session), sign in there and enable **Stay signed in**. Cookies are stored under the app data directory (`assist-webview/`) so later assists reuse that session.
3. With **autoclick** enabled (Settings, default on), the assist waits for the free download button countdown then clicks **Mod Manager Download** (preferred) or **Slow Download** (autoclick is skipped on login pages)
4. `nxm://` keys stage via the API; Slow Download archives are imported when the assist window finishes saving the file
5. Or use **Import archive** on the Mods tab for a manual zip/7z/rar
6. **Collections:** Install queues every required mod (optional mods if enabled) and opens Download Assist one-by-one with the same autoclick flow; failed mods are skipped and summarized at the end. Use **Cancel** on the status banner to stop the queue.
7. Settings → **Clear Nexus website session** deletes the assist WebView profile if you need to re-authenticate

Autoclick only runs in the user-opened assist window and only clicks Nexus’s free download controls after they are enabled — it does not mint Premium CDN links without credentials.

### `nxm://` handler

- **Linux:** the bundled desktop entry declares `MimeType=x-scheme-handler/nxm`. After installing the package, browsers can open “Download with manager” links in Nexus Manager.
- **Windows:** the NSIS installer / deep-link plugin registers the `nxm` protocol for the app.

## Data layout

Paths use the OS conventions via the `directories` crate:

| Purpose | Linux (XDG) | Windows |
|---------|-------------|---------|
| Config (`config.toml`) | `~/.config/nexus-manager/` | `%APPDATA%\nexusmanager\nexus-manager\` |
| Staged mods / load order | `~/.local/share/nexus-manager/` | `%APPDATA%\nexusmanager\nexus-manager\` (data) |
| Download cache | `~/.cache/nexus-manager/` | `%LOCALAPPDATA%\nexusmanager\nexus-manager\cache\` |

Exact folder names follow `ProjectDirs` (`dev` / `nexusmanager` / `nexus-manager`).

## License

AGPL-3.0-or-later (required by `lib_game_detector` on Linux).

## Not in MVP+

FOMOD wizards, LOOT sorting, BSA/BA2 packing, Proton script-extender shims, multi-profile workflows.

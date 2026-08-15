# Nexus Manager

Linux-native mod manager for [Nexus Mods](https://www.nexusmods.com/).

**MVP+ features**

- Detect installed games (Steam, Heroic, Lutris, Bottles) via `lib_game_detector`
- Manage supported titles: **Stardew Valley**, **Baldur's Gate 3**, **Cyberpunk 2077**
- Search mods & Collections on Nexus (GraphQL v2 + REST v1)
- **Premium:** one-click API downloads and Collection batch install
- **Free:** visible **Download Assist** WebView on the Nexus file page (persistent website session in-app), optional **autoclick** (Nexus Fast Download–style) on Mod Manager / Slow Download when the button is enabled, keyed `nxm://` staging, local archive import, and **Collection install** via a sequential Download Assist queue
- Hardlink/symlink deploy with basic load order
- Handle `nxm://` deep links from the Nexus website

## Requirements

- Linux (distro-agnostic; XDG paths)
- Rust toolchain + Node.js 20+
- System packages typically needed for Tauri: `webkit2gtk`, `libayatana-appindicator`, etc. (see [Tauri Linux prerequisites](https://v2.tauri.app/start/prerequisites/))
- A Nexus Mods account and **personal API key**
- **Nexus Premium** for one-click `download_link.json` without a site-issued key (and API Collection batch install)
- Optional: `unrar` or `unar` on `PATH` to extract `.rar` mod archives (e.g. `pacman -S unrar`)

## Setup

```bash
npm install
npm run tauri dev
```

Production build:

```bash
npm run tauri build
```

### API key

1. Open [Nexus API Access](https://www.nexusmods.com/users/myaccount?tab=api)
2. Create / copy a personal API key
3. Paste it in **Setup** inside the app

The key is stored in the OS keyring when available, otherwise in `~/.config/nexus-manager/nexus_api_key` (mode `0600`).

### Free-account downloads

1. Browse a mod file → **Download Assist**
2. A visible Nexus window opens. The **API key does not log you into the website** — the first time (or after clearing the session), sign in there and enable **Stay signed in**. Cookies are stored under `~/.local/share/nexus-manager/assist-webview/` so later assists reuse that session.
3. With **autoclick** enabled (Settings, default on), the assist waits for the free download button countdown then clicks **Mod Manager Download** (preferred) or **Slow Download** (autoclick is skipped on login pages)
4. `nxm://` keys stage via the API; Slow Download archives are imported when the assist window finishes saving the file
5. Or use **Import archive** on the Mods tab for a manual zip/7z/rar
6. **Collections:** Install queues every required mod (optional mods if enabled) and opens Download Assist one-by-one with the same autoclick flow; failed mods are skipped and summarized at the end. Use **Cancel** on the status banner to stop the queue.
7. Settings → **Clear Nexus website session** deletes the assist WebKit profile if you need to re-authenticate

Autoclick only runs in the user-opened assist window and only clicks Nexus’s free download controls after they are enabled — it does not mint Premium CDN links without credentials.

### `nxm://` handler

The bundled desktop entry declares `MimeType=x-scheme-handler/nxm`. After installing the package, browsers can open “Download with manager” links in Nexus Manager.

## Data layout

| Path | Purpose |
|------|---------|
| `~/.config/nexus-manager/config.toml` | Managed games & settings |
| `~/.local/share/nexus-manager/{game}/mods/` | Staged mod contents |
| `~/.local/share/nexus-manager/{game}/loadorder.json` | Enable flags & order |
| `~/.cache/nexus-manager/downloads/` | Temporary archives |

## License

AGPL-3.0-or-later (required by `lib_game_detector`).

## Not in MVP+

FOMOD wizards, LOOT sorting, BSA/BA2 packing, Proton script-extender shims, multi-profile workflows.

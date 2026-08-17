# Emperor Mod Manager

Desktop mod manager for [Nexus Mods](https://www.nexusmods.com/), Thunderstore, and mod.io on **Linux** and **Windows**.

**MVP+ features**

- Detect installed games (Linux: Steam / Heroic / Lutris / Bottles via `lib_game_detector`; Windows: Steam registry + libraries, optional Heroic)
- Manage supported titles: **Stardew Valley**, **Baldur's Gate 3**, **Cyberpunk 2077**, **The Witcher 3**, **Days Gone**, **S.T.A.L.K.E.R. 2: Heart of Chornobyl**, **Palworld**, **Hogwarts Legacy**, **Ready or Not**, **Subnautica 2**, **Marvel Rivals**, **Deep Rock Galactic**, **Pavlov VR**, **Oblivion Remastered**, **Helldivers 2**, **Warhammer 40,000: Darktide**, **Dark Souls** (PTDE), **Dark Souls Remastered**, **Dark Souls 2**, **Dark Souls 3**, **Elden Ring**, **Resident Evil 7**, **Village**, **Requiem**, **2/3/4 Remake**, **Monster Hunter Wilds**, **Monster Hunter Rise**, **Monster Hunter: World**, **Kingdom Come: Deliverance** 1 and 2, **Mount & Blade II: Bannerlord**, **Mount & Blade: Warband**, **The Forest**, **My Summer Car**, **7 Days to Die**, **RimWorld**, **No Man's Sky**, **SnowRunner**, **Space Engineers**, **Skyrim Special Edition**, **Fallout 4**, **Starfield**, **Blade & Sorcery**, **BONELAB**, **BONEWORKS**, a **generic Unreal Engine** pipeline for other UE4/UE5 installs, and **Unity/BepInEx** titles (**Lethal Company**, **Valheim**, **Risk of Rain 2**, **Among Us**, **Content Warning**, **GTFO**, **Timberborn**, **Cult of the Lamb**, **Against the Storm**, **ROUNDS**, **Sons of the Forest**, **Subnautica**, **Subnautica: Below Zero**, **Schedule I**, **R.E.P.O.**, **PEAK**, **H3VR**, **ULTRAKILL**, **ATLYSS**, **Dyson Sphere Program**, **Inscryption**, **Hollow Knight: Silksong**, plus generic BepInEx)
- Days Gone: `.pak` mods deploy to `BendGame/Saved/Paks` (Windows `%LOCALAPPDATA%` or Steam Proton compatdata). First deploy renames `startuppackages.pak` → `startuppackages_modsenabled.pak` when needed so custom paks load.
- Darktide: mods deploy under `<game>/mods/`; enabled names are written to `mods/mod_load_order.txt`. Deploy runs `tools/dtkit-patch --patch bundle` (idempotent enable; not `--toggle`). Install the Darktide Mod Loader first so the patcher binary is present.
- Witcher 3: `mods/`, `dlc/`, and `bin/` deploy at the install root; enabled packs are written to `mods.settings` (Windows Documents or Proton `compatdata/292030`). Script Merger is not included.
- Helldivers 2: patch archives deploy to `data/` and are renamed to sequential `.patch_N` (plus `.stream` / `.gpu_resources`) per archive id from load order. `.dl-bin` files go to `data/game/`.
- Kingdom Come: Deliverance 1/2: mods deploy under `Mods/<ModName>/`; enabled names are written to `Mods/mod_order.txt` (KCD2 treats this as a whitelist).
- Monster Hunter: World deploys to `nativePC/` (Stracker's Loader). Rise and Wilds use the same RE Engine layout as the Resident Evil titles (`natives/`, `reframework/`, `pak_mods/`).
- Bannerlord modules deploy to `Modules/`; the app warns if BLSE is missing. Warband uses the same `Modules/` layout.
- Blade & Sorcery SDK mods (`manifest.json`) deploy to `*_Data/StreamingAssets/Mods/`.
- RE Engine Resident Evil titles deploy `natives/`, `reframework/`, and `pak_mods/` into the game install (plus REF injectors like `dinput8.dll`). Install [REFramework](https://github.com/praydog/reframework) and enable **Loose File Loader** for loose-file mods; this app does not perform Fluffy-style PAK invalidation.
- FromSoftware titles deploy under `<Game>/mods/<ModName>/` (resolves the Steam `Game/` subfolder when present). Launch with an external injector such as [me3](https://github.com/garyttierney/me3), Mod Engine 2, Elden Mod Loader, or legacy Mod Engine — this app does not install or launch those tools.
- Unity/BepInEx titles deploy plugins under `BepInEx/plugins/` (Doorstop / BepInExPack trees deploy to the install root). Install BepInEx or a Thunderstore BepInExPack first; the app warns when the loader is missing.
- MelonLoader titles (BONELAB, BONEWORKS) deploy under `Mods/` (loader packs to the install root). Install MelonLoader first.
- 7 Days to Die and RimWorld wrap mod folders under `Mods/`. No Man's Sky uses `GAMEDATA/MODS` (not legacy `PCBANKS/MODS`). Space Engineers local mods go to the user-data `Mods` folder (`%APPDATA%` or Proton compatdata). SnowRunner paks go to `preload/paks/client`.
- Creation Engine titles (Skyrim Special Edition, Fallout 4, Starfield) overlay into `Data/` and rewrite `Plugins.txt`. Oblivion Remastered maps ESPs to `OblivionRemastered/Content/Dev/ObvData/Data` and UE paks to `Content/Paks/~mods`. Script extenders (SKSE/F4SE/SFSE/OBSE), LOOT, and FOMOD are not installed by this app.
- Search mods & Collections on Nexus (GraphQL v2 + REST v1). **Browse** can merge **Nexus**, **Thunderstore**, and **mod.io** in one list when each source is configured (source badges; All / Nexus / Thunderstore / mod.io filter). Thunderstore and mod.io installs resolve soft dependencies automatically and keep a **single newest version** of each package. **Collections** merge Nexus Collections with Thunderstore **Modpacks**; paste an r2modman/Gale **profile code** to import a Thunderstore profile. Installed collections/profiles are tracked per game so you can filter or uninstall one set without removing shared requirements.
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
- Optional [mod.io API key](https://mod.io/me/access) (read-only) to browse/download from mod.io for managed games with a mod.io game ID

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

### mod.io API key

1. Open [mod.io Access](https://mod.io/me/access) and create / copy an API key
2. Paste it under **Setup → mod.io** in the app
3. Set a **mod.io game ID** on the managed game (Library → Catalog & deploy settings), or rely on a seeded ID when one exists

The key is stored like the Nexus key (`modio-api-key` in the keyring, or `modio_api_key` in the config directory). Auth is **API key only** — no OAuth, Steam login, upload, subscribe, rate, or comments.

### Free-account downloads

1. Browse a mod file → **Download Assist**
2. A visible Nexus window opens. The **API key does not log you into the website** — the first time (or after clearing the session), sign in there and enable **Stay signed in**. Cookies are stored under the app data directory (`assist-webview/`) so later assists reuse that session.
3. With **autoclick** enabled (Settings, default on), the assist waits for the free download button countdown then clicks **Mod Manager Download** (preferred) or **Slow Download** (autoclick is skipped on login pages)
4. `nxm://` keys stage via the API; Slow Download archives are imported when the assist window finishes saving the file
5. Or use **Import archive** on the Mods tab for a manual zip/7z/rar
6. **Collections:** Install queues every required mod (optional mods if enabled) and opens Download Assist one-by-one with the same autoclick flow; failed mods are skipped and summarized at the end. Completed files are recorded as an installed collection so you can uninstall that set later. Use **Cancel** on the status banner to stop the queue.
7. Settings → **Clear Nexus website session** deletes the assist WebView profile if you need to re-authenticate

Autoclick only runs in the user-opened assist window and only clicks Nexus’s free download controls after they are enabled — it does not mint Premium CDN links without credentials.

### `nxm://` handler

- **Linux:** the bundled desktop entry declares `MimeType=x-scheme-handler/nxm`. After installing the package, browsers can open “Download with manager” links in Emperor Mod Manager.
- **Windows:** the NSIS installer / deep-link plugin registers the `nxm` protocol for the app.

## Data layout

Paths use the OS conventions via the `directories` crate:

| Purpose | Linux (XDG) | Windows |
|---------|-------------|---------|
| Config (`config.toml`) | `~/.config/emperor-mod-manager/` | `%APPDATA%\emperormodmanager\emperor-mod-manager\` |
| Staged mods / load order | `~/.local/share/emperor-mod-manager/` | `%APPDATA%\emperormodmanager\emperor-mod-manager\` (data) |
| Download cache | `~/.cache/emperor-mod-manager/` | `%LOCALAPPDATA%\emperormodmanager\emperor-mod-manager\cache\` |

Exact folder names follow `ProjectDirs` (`dev` / `emperormodmanager` / `emperor-mod-manager`).

### Recovering data from Nexus Manager

Emperor copies staged game data from the legacy `nexus-manager` app-data location at startup
when its matching game folder is empty, then rewrites the saved staging paths. The legacy data
is never deleted automatically because an existing game deployment may still symlink to it.
Use **Settings → Mod recovery** to scan or rerun recovery, deploy each recovered game, and only
then remove the old data directory manually.

## License

AGPL-3.0-or-later (required by `lib_game_detector` on Linux).

## Not in MVP+

FOMOD wizards, LOOT sorting, BSA/BA2 packing, Proton script-extender shims, multi-profile workflows, MelonLoader, mod.io OAuth / upload / ratings, bundling a shared app-wide mod.io key. Bethesda titles, Mass Effect (ME3Tweaks), and Ghost Recon Breakpoint (AnvilToolkit forge repack) are not supported.

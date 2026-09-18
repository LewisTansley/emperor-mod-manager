# Emperor Mod Manager

Desktop mod manager for [Nexus Mods](https://www.nexusmods.com/), Thunderstore, and mod.io on **Linux** and **Windows**.

**MVP+ features**

- Detect installed games (Linux: Steam / Heroic / Lutris / Bottles via `lib_game_detector`; Windows: Steam registry + libraries, optional Heroic)
- Manage supported titles: **Stardew Valley**, **Baldur's Gate 3**, **Cyberpunk 2077**, **The Witcher 3**, **Days Gone**, **S.T.A.L.K.E.R. 2: Heart of Chornobyl**, **Palworld**, **Hogwarts Legacy**, **Ready or Not**, **Subnautica 2**, **Marvel Rivals**, **Deep Rock Galactic**, **Pavlov VR**, **Oblivion Remastered**, **Helldivers 2**, **Warhammer 40,000: Darktide**, **Dark Souls** (PTDE), **Dark Souls Remastered**, **Dark Souls 2**, **Dark Souls 3**, **Elden Ring**, **Resident Evil 7**, **Village**, **Requiem**, **2/3/4 Remake**, **Monster Hunter Wilds**, **Monster Hunter Rise**, **Monster Hunter: World**, **Kingdom Come: Deliverance** 1 and 2, **Mount & Blade II: Bannerlord**, **Mount & Blade: Warband**, **The Forest**, **My Summer Car**, **7 Days to Die**, **RimWorld**, **No Man's Sky**, **SnowRunner**, **Space Engineers**, **The Sims 3**, **The Sims 4**, **Skyrim Special Edition**, **Fallout 4**, **Starfield**, **Blade & Sorcery**, **BONELAB**, **BONEWORKS**, a **generic Unreal Engine** pipeline for other UE4/UE5 installs, and **Unity/BepInEx** titles (**Lethal Company**, **Valheim**, **Risk of Rain 2**, **Among Us**, **Content Warning**, **GTFO**, **Timberborn**, **Cult of the Lamb**, **Against the Storm**, **ROUNDS**, **Sons of the Forest**, **Subnautica**, **Subnautica: Below Zero**, **Schedule I**, **R.E.P.O.**, **PEAK**, **H3VR**, **ULTRAKILL**, **ATLYSS**, **Dyson Sphere Program**, **Inscryption**, **Hollow Knight: Silksong**, plus generic BepInEx)
- Days Gone: `.pak` mods deploy to `BendGame/Saved/Paks` (Windows `%LOCALAPPDATA%` or Steam Proton compatdata). First deploy renames `startuppackages.pak` → `startuppackages_modsenabled.pak` when needed so custom paks load.
- Darktide: mods deploy under `<game>/mods/`; enabled names are written to `mods/mod_load_order.txt`. Deploy runs `tools/dtkit-patch --patch bundle` (idempotent enable; not `--toggle`). Install the Darktide Mod Loader first so the patcher binary is present.
- Witcher 3: `mods/`, `dlc/`, and `bin/` deploy at the install root; enabled packs are written to `mods.settings` (Windows Documents or Proton `compatdata/292030`). Script Merger is not included.
- Helldivers 2: patch archives deploy to `data/` and are renamed to sequential `.patch_N` (plus `.stream` / `.gpu_resources`) per archive id from load order. `.dl-bin` files go to `data/game/`. Mods packaged for **Arsenal** / **HD2MM** (a `manifest.json` at the mod root, V1 or legacy) expose their **Options** on the mod row: options are checkboxes, sub-options are a pick-one group, and only the selected include folders deploy. Files at the mod root always deploy; folders the manifest never includes never do. New mods start with every option on and the first sub-option chosen, and your picks survive a mod update. Deploy again after changing options.
- Kingdom Come: Deliverance 1/2: mods deploy under `Mods/<ModName>/`; enabled names are written to `Mods/mod_order.txt` (KCD2 treats this as a whitelist).
- Monster Hunter: World deploys to `nativePC/` (Stracker's Loader). Rise and Wilds use the same RE Engine layout as the Resident Evil titles (`natives/`, `reframework/`, `pak_mods/`).
- Bannerlord modules deploy to `Modules/`; the app warns if BLSE is missing. Warband uses the same `Modules/` layout.
- Blade & Sorcery SDK mods (`manifest.json`) deploy to `*_Data/StreamingAssets/Mods/`.
- RE Engine Resident Evil titles deploy `natives/`, `reframework/`, and `pak_mods/` into the game install (plus REF injectors like `dinput8.dll`). Install [REFramework](https://github.com/praydog/reframework) and enable **Loose File Loader** for loose-file mods; this app does not perform Fluffy-style PAK invalidation.
- FromSoftware titles deploy under `<Game>/mods/<ModName>/` (resolves the Steam `Game/` subfolder when present). Launch with an external injector such as [me3](https://github.com/garyttierney/me3), Mod Engine 2, Elden Mod Loader, or legacy Mod Engine — this app does not install or launch those tools.
- Unity/BepInEx titles deploy plugins under `BepInEx/plugins/` (Doorstop / BepInExPack trees deploy to the install root, and packages shipping `BepInEx/patchers`, `config`, or `monomod` merge into those folders). Install BepInEx or a Thunderstore BepInExPack first; the app warns when the loader is missing. When a BepInEx.GUI console patcher is deployed, the console is switched on in `BepInEx.cfg` so its window actually appears.
- MelonLoader titles (BONELAB, BONEWORKS) deploy under `Mods/` (loader packs to the install root). Install MelonLoader first.
- 7 Days to Die and RimWorld wrap mod folders under `Mods/`. No Man's Sky uses `GAMEDATA/MODS` (not legacy `PCBANKS/MODS`). Space Engineers local mods go to the user-data `Mods` folder (`%APPDATA%` or Proton compatdata). SnowRunner paks go to `preload/paks/client`.
- The Sims 4: mods deploy to `Documents/Electronic Arts/The Sims 4/Mods/<ModName>/` (Windows Documents, Proton `compatdata/1222670`, or a Wine prefix for EA App installs). `Mods/Resource.cfg` is written when missing; a `.ts4script` nested deeper than one folder is flattened into the mod folder so the game can load it. Only `.package` and `.ts4script` files deploy, and `localthumbcache.package` is deleted after each deploy. Enable **Script Mods Allowed** in Game Options yourself — the app only warns when it is off.
- The Sims 3: `.package` mods deploy to `Documents/Electronic Arts/The Sims 3/Mods/Packages/<ModName>/`; archives shipping their own `Mods/`, `Overrides/`, or `DCCache/` tree keep that layout. The framework `Resource.cfg` is written when missing, and the regenerable caches (`scriptCache`, `CASPartCache`, `compositorCache`, `simCompositorCache`, `socialCache`, `WorldCaches`) are cleared after deploy. `.sims3pack` files land in `Downloads/` and still need the Sims 3 Launcher to install them — uninstalling the mod here will not undo that.
- Single-file mods (`.package`, `.ts4script`, `.dbc`, `.sims3pack`) can be imported directly, not only as zip/7z/rar archives.
- Creation Engine titles (Skyrim Special Edition, Fallout 4, Starfield) overlay into `Data/` and rewrite `Plugins.txt`. Oblivion Remastered maps ESPs to `OblivionRemastered/Content/Dev/ObvData/Data` and UE paks to `Content/Paks/~mods`. Script extenders (SKSE/F4SE/SFSE/OBSE), LOOT, and FOMOD are not installed by this app.
- Search mods & Collections on Nexus (GraphQL v2 + REST v1). **Browse** can merge **Nexus**, **Thunderstore**, and **mod.io** in one list when each source is configured (source badges; All / Nexus / Thunderstore / mod.io filter). Thunderstore and mod.io installs resolve soft dependencies automatically and keep a **single version** of each package: dependencies install at the version they are pinned to (highest pin wins), so a mod set loads with the builds it was tested against. **Collections** merge Nexus Collections with Thunderstore **Modpacks**; paste an r2modman/Gale **profile code** to import a Thunderstore profile. Installed collections/profiles are tracked per game so you can filter or uninstall one set without removing shared requirements.
- **Premium:** one-click API downloads and Collection batch install
- **Free:** visible **Download Assist** WebView on the Nexus file page (persistent website session in-app), optional **autoclick** (Nexus Fast Download–style) on Mod Manager / Slow Download when the button is enabled, keyed `nxm://` staging, local archive import, and **Collection install** via a sequential Download Assist queue
- Hardlink deploy with copy fallback across drives (basic load order)
- Handle `nxm://` deep links from the Nexus website

## Install

### Arch Linux (also CachyOS, EndeavourOS, Manjaro)

```bash
curl -fsSL https://raw.githubusercontent.com/LewisTansley/emperor-mod-manager/main/scripts/install-arch.sh | bash
```

That downloads the latest release, builds a pacman package from [`packaging/arch/PKGBUILD`](packaging/arch/PKGBUILD) and installs it, so the app shows up in your menu and `nxm://` links work right away. It needs `base-devel` (`sudo pacman -S --needed base-devel`) and asks for sudo only at the pacman step.

Prefer not to pipe a script into your shell? The same thing by hand:

```bash
git clone https://github.com/LewisTansley/emperor-mod-manager.git
cd emperor-mod-manager/packaging/arch
makepkg -si
```

Pin a version with `EMM_VERSION=0.4.0` before the installer, update by re-running it, and uninstall with `sudo pacman -R emperor-mod-manager-bin`. The in-app updater only replaces an AppImage, so a pacman install should be updated through pacman.

### Other Linux distros

Grab the `.deb` or `.AppImage` from the [latest release](https://github.com/LewisTansley/emperor-mod-manager/releases/latest). Each release also ships `emperor-mod-manager-<version>-x86_64.tar.gz` (binary, desktop entry, icons) with a `SHA256SUMS` file if you package it yourself.

### Windows

Run the `..._x64-setup.exe` installer from the [latest release](https://github.com/LewisTansley/emperor-mod-manager/releases/latest); it bootstraps WebView2 and registers the `nxm` protocol.

## Requirements

- Linux or Windows 10+
- Rust toolchain + Node.js 20+ (only to build from source)
- **Linux:** system packages typically needed for Tauri: `webkit2gtk`, `libayatana-appindicator`, etc. (see [Tauri Linux prerequisites](https://v2.tauri.app/start/prerequisites/))
- **Windows:** [WebView2](https://developer.microsoft.com/microsoft-edge/webview2/) (installer can bootstrap it)
- A Nexus Mods account and **personal API key**
- **Nexus Premium** for one-click `download_link.json` without a site-issued key (and API Collection batch install)
- Optional [mod.io API key](https://mod.io/me/access) (read-only) to browse/download from mod.io for managed games with a mod.io game ID

## Setup (from source)

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

- **Linux:** the bundled desktop entry declares `MimeType=x-scheme-handler/nxm`. After installing the package, browsers can open “Download with manager” links in Emperor Mod Manager. The Arch package installs that entry system-wide, and pacman's `desktop-file-utils` hook refreshes the MIME database for you.
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
is never deleted automatically because an existing game deployment may still hardlink to it.
Use **Settings → Mod recovery** to scan or rerun recovery, deploy each recovered game, and only
then remove the old data directory manually.

## License

AGPL-3.0-or-later (required by `lib_game_detector` on Linux).

## Not in MVP+

FOMOD wizards, LOOT sorting, BSA/BA2 packing, Proton script-extender shims, multi-profile workflows, MelonLoader, mod.io OAuth / upload / ratings, bundling a shared app-wide mod.io key. Bethesda titles, Mass Effect (ME3Tweaks), and Ghost Recon Breakpoint (AnvilToolkit forge repack) are not supported.

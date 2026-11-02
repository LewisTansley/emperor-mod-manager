# Emperor Mod Manager

Desktop mod manager for [Nexus Mods](https://www.nexusmods.com/), Thunderstore, and mod.io — on **Linux** and **Windows**.

Browse, download, and deploy mods for dozens of games across Unity/BepInEx, Unreal, RE Engine, FromSoftware, Bethesda/Creation Engine, and many more titles. Detects installs from Steam, Heroic, Lutris, and Bottles on Linux, and Steam (plus optional Heroic) on Windows.

## Highlights

- **One browse list** for Nexus, Thunderstore, and mod.io (filter by source when you want)
- **Collections and modpacks** — install Nexus Collections, Thunderstore Modpacks, or paste an r2modman/Gale profile code
- **Deploy with load order** — hardlinks when possible, copy fallback across drives
- **Nexus Premium** — one-click API downloads and Collection batch install
- **Download Assist** — free Nexus accounts download in-app (optional autoclick), with `nxm://` staging and local archive import
- **Browser deep links** — “Download with manager” (`nxm://`) opens in Emperor after install
- Soft dependency resolution on Thunderstore and mod.io, with a single pinned version per package

## Install

### Arch Linux (also CachyOS, EndeavourOS, Manjaro)

```bash
curl -fsSL https://raw.githubusercontent.com/LewisTansley/emperor-mod-manager/main/scripts/install-arch.sh | bash
```

That downloads the latest release, builds a pacman package, and installs it so the app appears in your menu and `nxm://` links work right away. You need `base-devel` (`sudo pacman -S --needed base-devel`); sudo is only asked at the pacman step.

Prefer not to pipe a script into your shell? Same result by hand:

```bash
mkdir -p "$HOME/.cache/emperor-mod-manager" && cd "$HOME/.cache/emperor-mod-manager"
curl -fsSL -o PKGBUILD https://github.com/LewisTansley/emperor-mod-manager/releases/latest/download/PKGBUILD
makepkg -si
```

Pin a version with `EMM_VERSION=0.4.0` before the installer. Uninstall with `sudo pacman -R emperor-mod-manager-bin`. After the first install, **Settings → Check for updates** builds and installs the new release (still needs `base-devel`, asks for your password once). You can also update by re-running this installer.

### Other Linux distros

Grab the `.deb` or `.AppImage` from the [latest release](https://github.com/LewisTansley/emperor-mod-manager/releases/latest). Each release also ships a `.tar.gz` with the binary, desktop entry, and icons, plus a `SHA256SUMS` file. Settings updates a `.deb` with apt, replaces an AppImage in place, or replaces a user-writable copy of the binary from that tarball.

### Windows

Run the `..._x64-setup.exe` installer from the [latest release](https://github.com/LewisTansley/emperor-mod-manager/releases/latest). It sets up WebView2 if needed and registers the `nxm` protocol so browser download links open in the app.

## Getting started

You need Linux or Windows 10+, a [Nexus Mods](https://www.nexusmods.com/) account, and a personal API key. Nexus Premium enables one-click downloads and Collection batch install; free accounts use Download Assist (below). An optional [mod.io API key](https://mod.io/me/access) unlocks browsing and downloading from mod.io for games that have a mod.io ID.

### Nexus API key

1. Open [Nexus API Access](https://www.nexusmods.com/users/myaccount?tab=api)
2. Create or copy a personal API key
3. Paste it under **Setup** in the app

The key is stored in your OS keyring when available, otherwise in the app config directory (mode `0600` on Linux).

### mod.io (optional)

1. Open [mod.io Access](https://mod.io/me/access) and create or copy an API key
2. Paste it under **Setup → mod.io**
3. Set a **mod.io game ID** on the managed game (Library → Catalog & deploy settings), or use a seeded ID when one exists

### Free-account downloads

1. Open a mod file and choose **Download Assist**
2. Sign in to Nexus in the assist window the first time (enable **Stay signed in**). Later assists reuse that session.
3. With **autoclick** on (Settings, default on), the assist waits for the free download button, then clicks **Mod Manager Download** or **Slow Download**
4. Or use **Import archive** on the Mods tab for a zip/7z/rar you already have
5. **Collections** queue each required mod through Download Assist one-by-one; use **Cancel** on the status banner to stop

**Settings → Clear Nexus website session** resets the assist login if you need to sign in again.

## Your data

| Purpose | Linux | Windows |
|---------|-------|---------|
| Config | `~/.config/emperor-mod-manager/` | `%APPDATA%\emperormodmanager\emperor-mod-manager\` |
| Staged mods / load order | `~/.local/share/emperor-mod-manager/` | `%APPDATA%\emperormodmanager\emperor-mod-manager\` |
| Download cache | `~/.cache/emperor-mod-manager/` | `%LOCALAPPDATA%\emperormodmanager\emperor-mod-manager\cache\` |

If you previously used the legacy Nexus Manager app, Emperor can copy staged game data at startup when the matching folder is empty. Use **Settings → Mod recovery** to scan or rerun recovery, deploy each recovered game, then remove the old data directory yourself when you are done.

## License

[AGPL-3.0-or-later](LICENSE)

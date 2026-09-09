//! Sync per-game tool profiles and Steam launch options.

use anyhow::{Context, Result};

use crate::config::{GameToolOverrides, ManagedGame, Paths, ToolGameState};

use super::{
    autohdr_vk::AutoHdrVkConfig,
    executables::{detect_executables, profile_name_for_game, steam_app_id_from_install},
    lsfg_vk::{LsfgVkConfig, LsfgVkProfile},
    steam_launch::{LaunchEnvBlock, LsfgLaunch, merge_launch_options, read_launch_options, write_launch_options},
};

pub fn sync_game_tools(paths: &Paths, game: &ManagedGame) -> Result<()> {
    let overrides = game.tool_overrides.clone().unwrap_or_default();
    let executables = resolve_executables(game, &overrides);
    sync_lsfg_vk(paths, game, &overrides, &executables)?;
    sync_autohdr_vk(paths, game, &overrides, &executables)?;
    sync_steam_launch(paths, game, &overrides)?;
    Ok(())
}

fn resolve_executables(game: &ManagedGame, overrides: &GameToolOverrides) -> Vec<String> {
    if !overrides.executables.is_empty() {
        return overrides.executables.clone();
    }
    detect_executables(&game.install_path, 8)
}

fn sync_lsfg_vk(
    paths: &Paths,
    game: &ManagedGame,
    overrides: &GameToolOverrides,
    executables: &[String],
) -> Result<()> {
    let conf_path = paths.lsfg_vk_config_file();
    let mut cfg = LsfgVkConfig::load(&conf_path)?;
    let profile_name = profile_name_for_game(&game.id);
    cfg.remove_profile(&profile_name);

    if overrides.lsfg_vk.enabled {
        let global = cfg.default_profile().clone();
        let mut profile = if overrides.lsfg_vk.use_global_settings {
            global
        } else {
            LsfgVkProfile::default()
        };
        profile.name = profile_name.clone();
        profile.active_in = executables.to_vec();
        cfg.profiles.push(profile);
    }
    cfg.save(&conf_path)?;
    Ok(())
}

fn sync_autohdr_vk(
    paths: &Paths,
    _game: &ManagedGame,
    overrides: &GameToolOverrides,
    executables: &[String],
) -> Result<()> {
    let conf_path = paths.autohdr_vk_config_file();
    let mut cfg = AutoHdrVkConfig::load(&conf_path)?;

    for exe in executables {
        cfg.remove_profile(exe);
    }

    if overrides.autohdr_vk.enabled {
        for exe in executables {
            let profile = cfg.profile_mut(exe);
            profile.enabled = true;
            if overrides.autohdr_vk.use_global_settings {
                profile.intensity = None;
                profile.color_intensity = None;
                profile.expansion_shape = None;
                profile.black_floor = None;
                profile.highlight_stretch = None;
            }
        }
    }
    cfg.save(&conf_path)?;
    Ok(())
}

fn sync_steam_launch(
    paths: &Paths,
    game: &ManagedGame,
    overrides: &GameToolOverrides,
) -> Result<()> {
    let app_id = steam_app_id_from_install(&game.install_path);
    let Some(app_id) = app_id else {
        return Ok(());
    };

    let existing = read_launch_options(&app_id).unwrap_or(None).unwrap_or_default();
    let profile_name = profile_name_for_game(&game.id);

    let block = LaunchEnvBlock {
        lsfg_vk: if overrides.lsfg_vk.enabled {
            Some(LsfgLaunch {
                config_path: paths.lsfg_vk_config_file().display().to_string(),
                profile: profile_name,
            })
        } else {
            None
        },
        autohdr_vk: overrides.autohdr_vk.enabled,
    };

    let merged = merge_launch_options(&existing, &block);
    if merged != existing {
        write_launch_options(&app_id, &merged)
            .with_context(|| format!("updating Steam launch options for {}", game.title))?;
    }
    Ok(())
}

pub fn set_game_tool_state(
    overrides: &mut GameToolOverrides,
    tool: &str,
    state: ToolGameState,
) {
    match tool {
        "lsfg_vk" => overrides.lsfg_vk = state,
        "autohdr_vk" => overrides.autohdr_vk = state,
        _ => {}
    }
}

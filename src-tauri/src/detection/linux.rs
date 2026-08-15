use lib_game_detector::get_detector;

use super::{finish_detected, DetectedGame};

fn launcher_label(source: &lib_game_detector::data::SupportedLaunchers) -> String {
    format!("{source}")
}

pub fn scan_games() -> Vec<DetectedGame> {
    let detector = get_detector();
    let games = detector.get_all_detected_games();
    let mut out = Vec::with_capacity(games.len());

    for game in games {
        let launcher = launcher_label(&game.source);
        let install_path = game
            .path_game_dir
            .as_ref()
            .map(|p| p.to_string_lossy().to_string());
        let cover_path = game
            .path_box_art
            .as_ref()
            .or(game.path_icon.as_ref())
            .map(|p| p.to_string_lossy().to_string());
        out.push(finish_detected(game.title, launcher, install_path, cover_path));
    }

    out.sort_by(|a, b| {
        b.supported
            .cmp(&a.supported)
            .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
    });
    out
}

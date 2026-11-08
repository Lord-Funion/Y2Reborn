//! Home and its customization page, using the shared ordered place identities.
use super::*;
/// Home (UI v4): the places a listener starts from, in the listener's order (Y2Reborn #53):
/// Audiobooks once the library holds a book (Beta 8.19, can be switched off), any Music lists
/// pinned, and Home's own places unless taken off (#52, #55). Now Playing stays first, Settings
/// last.
pub(crate) fn home_rows(s: &Settings, tracks: &[Track]) -> Vec<Item> {
    let has_books = s.audiobooks_on_home && tracks.iter().any(|t| t.book && t.online);
    let mut rows = vec![row("Now Playing", "now_playing", "")];
    rows.extend(
        s.home_places()
            .into_iter()
            .filter(|(key, _)| s.home_place_shown(key, has_books))
            .map(|(key, label)| row(label, key, "")),
    );
    rows.push(row("Settings", "settings", ""));
    rows
}

/// Customize > Home Menu: every place Home can show, in Home's order, each switched on or off; hold
/// Select to move one (Y2Reborn #53).
pub(super) fn home_menu(ui: &Ui, m: &AppModel) -> Page {
    let carried = ui.home_moving.map(|c| c.to);
    let rows = ui
        .home_menu_places(m)
        .into_iter()
        .enumerate()
        .map(|(place, (key, label))| {
            let (row_key, on) = if key == "audiobooks" {
                ("home_audiobooks".to_owned(), m.settings.audiobooks_on_home)
            } else if HOME_PINNABLE.iter().any(|(k, _)| *k == key) {
                (
                    format!("home_pin:{key}"),
                    m.settings.home_pins.iter().any(|p| p == key),
                )
            } else {
                (format!("home_show:{key}"), m.settings.home_shows(key))
            };
            let moving = if carried == Some(place) { "Moving" } else { "" };
            row(label, &row_key, moving).with_kind(RowKind::Toggle(on))
        });
    let mut page = Page::rows(rows.collect());
    page.status = Some("Hold the centre button on an item to move it.".into());
    page
}

//! Settings > Customize: how Reborn behaves, in three short pages (owner, 9.2.10): what Home shows
//! (Home Menu), how the library lists look and end (Browsing) and how the wheel and buttons answer
//! (Wheel & Buttons). They are Customize with the navigation filter naming the page, so a session
//! saved here reads in older versions. A setting with three or more choices opens a picker
//! ("choose:<setting>"); on/off is a switch; two named choices step on Select. The line under each
//! page says what the setting in focus does.
use crate::pages::{home_summary, row, Page};
use crate::RowKind;
use reborn_core::{
    AlbumOrder, AlbumView, AppModel, ArtistAlbums, ArtistsList, Effect, Haptics, ScreenOffButtons,
    WheelClick, WrapAround,
};

/// The navigation filters of Customize's pages.
pub(crate) const BROWSING: &str = "browsing";
pub(crate) const WHEEL: &str = "wheel";
/// A picker's filter, and its rows' keys: "choose:<setting>", "set:<setting>:<index>".
const CHOOSE: &str = "choose:";
const SET: &str = "set:";

/// The page Customize shows for its navigation filter.
pub(crate) fn page(m: &AppModel) -> Page {
    let filter = m.navigation.filter.as_str();
    if let Some(setting) = filter.strip_prefix(CHOOSE) {
        return picker(m, setting);
    }
    let rows = match filter {
        BROWSING => vec![
            row("Cover Flow", "album_view", "").with_kind(RowKind::Toggle(
                m.settings.album_view == AlbumView::CoverFlow,
            )),
            row(
                "Sort Albums",
                "album_order",
                sort_albums(m.settings.album_order),
            ),
            row(
                "Artist's Albums",
                "choose:artist_albums",
                current(m, "artist_albums"),
            )
            .with_kind(RowKind::Chevron),
            row(
                "Artists List",
                "artists_list",
                artists_list(m.settings.artists_list),
            ),
            row(
                "Lists Wrap Around",
                "choose:wrap_around",
                current(m, "wrap_around"),
            )
            .with_kind(RowKind::Chevron),
        ],
        WHEEL => vec![
            row(
                "Wheel on Now Playing",
                "now_playing_wheel",
                m.settings.now_playing_wheel.label(),
            ),
            row("Wheel Click", "wheel_click", "")
                .with_kind(RowKind::Toggle(m.settings.wheel_click == WheelClick::On)),
            row("Haptics", "choose:haptics", current(m, "haptics")).with_kind(RowKind::Chevron),
            row(
                "Buttons While Screen Off",
                "choose:screen_off_buttons",
                current(m, "screen_off_buttons"),
            )
            .with_kind(RowKind::Chevron),
        ],
        _ => vec![
            row("Home Menu", "home_menu", home_summary(m)).with_kind(RowKind::Chevron),
            row("Browsing", "customize:browsing", "").with_kind(RowKind::Chevron),
            row("Wheel & Buttons", "customize:wheel", "").with_kind(RowKind::Chevron),
        ],
    };
    let mut page = Page::rows(rows);
    let focused = page.rows.get(m.navigation.focus).map(|r| r.key.as_str());
    page.status = focused.and_then(hint).map(str::to_owned);
    page
}

/// The title of a Customize page.
pub(crate) fn title(filter: &str) -> &'static str {
    match filter {
        BROWSING => "Browsing",
        WHEEL => "Wheel & Buttons",
        f => f
            .strip_prefix(CHOOSE)
            .and_then(|setting| choices(&AppModel::default(), setting))
            .map_or("Customize", |(name, _)| name),
    }
}

/// What the setting behind a row does, in one line.
fn hint(key: &str) -> Option<&'static str> {
    Some(match key {
        "home_menu" => "Which places Home shows, and in which order.",
        "customize:browsing" => "How albums and artists are listed, and how lists end.",
        "customize:wheel" => "What the wheel and buttons do, and how they answer.",
        "album_view" => "Albums and audiobooks as covers sliding past, or as a list.",
        "album_order" => "Albums A to Z by title, or each artist's albums together.",
        "choose:artist_albums" => {
            "How an artist's albums are ordered, on their page and in Albums by artist."
        }
        "artists_list" => "Everyone who sings on a song, or only each album's artist.",
        "choose:wrap_around" => "Whether a list goes on at the top after its last row.",
        "now_playing_wheel" => "What turning the wheel does on Now Playing.",
        "wheel_click" => "A soft tick on each wheel step while no music plays.",
        "choose:haptics" => "A short buzz on each button press.",
        "choose:screen_off_buttons" => {
            "What still works with the screen off. Power and volume always do."
        }
        _ => return None,
    })
}

fn sort_albums(order: AlbumOrder) -> &'static str {
    match order {
        AlbumOrder::Title => "By Title",
        AlbumOrder::Artist => "By Artist",
    }
}

fn artists_list(list: ArtistsList) -> &'static str {
    match list {
        ArtistsList::All => "All Artists",
        ArtistsList::AlbumArtists => "Album Artists Only",
    }
}

/// One choice of a picker: its name, what it means, whether it is the setting now and the
/// effect of choosing it.
struct Choice {
    name: &'static str,
    meaning: &'static str,
    chosen: bool,
    effect: Effect,
}

/// The pickers: the setting's name and its choices.
fn choices(m: &AppModel, setting: &str) -> Option<(&'static str, Vec<Choice>)> {
    let s = &m.settings;
    let choice = |name, meaning, chosen, effect| Choice {
        name,
        meaning,
        chosen,
        effect,
    };
    Some(match setting {
        "wrap_around" => (
            "Lists Wrap Around",
            [
                (WrapAround::Off, "Off", "Every list stops at its ends."),
                (
                    WrapAround::Menus,
                    "Menus Only",
                    "Home, Music and Settings go round; the library stops at its ends.",
                ),
                (
                    WrapAround::Everywhere,
                    "Everywhere",
                    "Every list goes round, the library too.",
                ),
            ]
            .map(|(v, name, meaning)| {
                choice(name, meaning, s.wrap_around == v, Effect::SetWrapAround(v))
            })
            .into(),
        ),
        "artist_albums" => (
            "Artist's Albums",
            [
                (
                    ArtistAlbums::OldestFirst,
                    "Oldest First",
                    "By release year, the first album at the top.",
                ),
                (
                    ArtistAlbums::NewestFirst,
                    "Newest First",
                    "By release year, the latest album at the top.",
                ),
                (ArtistAlbums::Title, "A to Z", "By album title."),
            ]
            .map(|(v, name, meaning)| {
                choice(
                    name,
                    meaning,
                    s.artist_albums == v,
                    Effect::SetArtistAlbums(v),
                )
            })
            .into(),
        ),
        "haptics" => (
            "Haptics",
            [
                (Haptics::Off, "Off", "No buzz."),
                (
                    Haptics::Light,
                    "Light",
                    "A light tick on each button press.",
                ),
                (
                    Haptics::Strong,
                    "Strong",
                    "A firm tick on each button press.",
                ),
            ]
            .map(|(v, name, meaning)| choice(name, meaning, s.haptics == v, Effect::SetHaptics(v)))
            .into(),
        ),
        "screen_off_buttons" => (
            "Buttons While Screen Off",
            [
                (
                    ScreenOffButtons::Locked,
                    "Locked",
                    "Nothing works: safe in a pocket.",
                ),
                (
                    ScreenOffButtons::Buttons,
                    "Play Buttons",
                    "Play/Pause, Previous and Next work; the wheel rests.",
                ),
                (
                    ScreenOffButtons::WheelAndButtons,
                    "Play Buttons and Wheel",
                    "The wheel changes the volume too, as on Now Playing.",
                ),
            ]
            .map(|(v, name, meaning)| {
                choice(
                    name,
                    meaning,
                    s.screen_off_buttons == v,
                    Effect::SetScreenOffButtons(v),
                )
            })
            .into(),
        ),
        _ => return None,
    })
}

/// The name of a picker setting's current choice, for its row.
fn current(m: &AppModel, setting: &str) -> &'static str {
    choices(m, setting)
        .and_then(|(_, all)| all.into_iter().find(|c| c.chosen))
        .map_or("", |c| c.name)
}

/// A picker: each choice checked when it is the setting, its meaning under the list.
fn picker(m: &AppModel, setting: &str) -> Page {
    let Some((_, all)) = choices(m, setting) else {
        return Page::default();
    };
    let meaning = all.get(m.navigation.focus).map(|c| c.meaning.to_owned());
    let mut page = Page::rows(
        all.iter()
            .enumerate()
            .map(|(i, c)| {
                row(c.name, &format!("{SET}{setting}:{i}"), "").with_kind(RowKind::Check(c.chosen))
            })
            .collect(),
    );
    page.status = meaning;
    page
}

/// Where a picker opens: on the current choice.
pub(crate) fn picker_focus(m: &AppModel, key: &str) -> usize {
    let setting = key.strip_prefix(CHOOSE).unwrap_or(key);
    choices(m, setting)
        .and_then(|(_, all)| all.iter().position(|c| c.chosen))
        .unwrap_or(0)
}

/// What Select on a picker's choice does.
pub(crate) fn chosen(m: &AppModel, key: &str) -> Option<Effect> {
    let (setting, index) = key.strip_prefix(SET)?.rsplit_once(':')?;
    let index: usize = index.parse().ok()?;
    choices(m, setting)?
        .1
        .into_iter()
        .nth(index)
        .map(|c| c.effect)
}

/// Whether a row key opens one of Customize's pages or pickers, and which filter.
pub(crate) fn opens(key: &str) -> Option<String> {
    key.strip_prefix("customize:")
        .map(str::to_owned)
        .or_else(|| key.starts_with(CHOOSE).then(|| key.to_owned()))
}

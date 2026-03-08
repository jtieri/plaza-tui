pub mod charts;
pub mod favorites;
pub mod help;
pub mod history;
pub mod login;
pub mod news;
pub mod now_playing;
pub mod profile;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum View {
    Login,
    NowPlaying,
    History,
    Favorites,
    Charts,
    News,
    Profile,
}

impl View {
    pub fn nav_items() -> &'static [(&'static str, View)] {
        &[
            ("\u{25b6} Now Playing", View::NowPlaying),
            ("  History", View::History),
            ("  Favorites", View::Favorites),
            ("  Charts", View::Charts),
            ("  News", View::News),
            ("  Profile", View::Profile),
        ]
    }
}

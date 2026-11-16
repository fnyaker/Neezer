//! The route table and the navigation stack.
//!
//! Every screen of the SPA's inventory is a [`Route`] here — declared in D7 so
//! the screens that implement them never edit this file or `screens/mod.rs`.
//! The stack keeps what a person expects from back/forward: a *history* of
//! entries, a forward stack that the next push clears, and the scroll offset of
//! each entry so returning to a long list lands where it was left
//! (`docs/agents/desktop.md`, D7 *Contract*).

use std::collections::HashMap;

/// Which half of Bibliothèque is open.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum LibraryTab {
    #[default]
    Playlists,
    Favorites,
    Artists,
    Albums,
    Uploads,
    Dislikes,
    Unavailable,
    Folders,
}

impl LibraryTab {
    pub fn title(self) -> &'static str {
        match self {
            LibraryTab::Playlists => "Playlists",
            LibraryTab::Favorites => "Favoris",
            LibraryTab::Artists => "Artistes",
            LibraryTab::Albums => "Albums",
            LibraryTab::Uploads => "Fichiers importés",
            LibraryTab::Dislikes => "Titres écartés",
            LibraryTab::Unavailable => "Titres indisponibles",
            LibraryTab::Folders => "Dossiers",
        }
    }
}

/// Which settings section is open. D14a and D14b own the screen bodies; the
/// variants are fixed now so the sidebar and the `g ,` route cannot drift.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SettingsSection {
    #[default]
    Playback,
    Appearance,
    Viz,
    Downloads,
    Discovery,
    Sources,
    Account,
    Users,
    Server,
    Transcribe,
    Help,
    Shortcuts,
}

impl SettingsSection {
    pub fn title(self) -> &'static str {
        match self {
            SettingsSection::Playback => "Lecture et son",
            SettingsSection::Appearance => "Apparence",
            SettingsSection::Viz => "Visualiseur",
            SettingsSection::Downloads => "Téléchargements",
            SettingsSection::Discovery => "Découverte",
            SettingsSection::Sources => "Sources",
            SettingsSection::Account => "Compte",
            SettingsSection::Users => "Utilisateurs",
            SettingsSection::Server => "Serveur",
            SettingsSection::Transcribe => "Transcription locale",
            SettingsSection::Help => "Assistance",
            SettingsSection::Shortcuts => "Raccourcis",
        }
    }

    /// The sections a non-admin sees, in the order the SPA lists them.
    pub const USER: [SettingsSection; 10] = [
        SettingsSection::Playback,
        SettingsSection::Appearance,
        SettingsSection::Viz,
        SettingsSection::Downloads,
        SettingsSection::Discovery,
        SettingsSection::Sources,
        SettingsSection::Account,
        SettingsSection::Transcribe,
        SettingsSection::Help,
        SettingsSection::Shortcuts,
    ];

    /// The two an administrator gets on top.
    pub const ADMIN: [SettingsSection; 2] = [SettingsSection::Users, SettingsSection::Server];
}

/// What a credits screen is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CreditKind {
    Track,
    Person,
}

/// One place the app can be.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Route {
    Login,
    Onboarding,
    Home,
    Search { query: String },
    Artist { id: String },
    Album { id: String },
    Playlist { id: String },
    Library { tab: LibraryTab },
    Podcasts,
    Show { id: String },
    Releases,
    Recap { year: Option<i32> },
    History,
    Genres,
    Credits { kind: CreditKind, id: String },
    Import { url: Option<String> },
    Party,
    Remote,
    NowPlaying,
    Queue,
    Viz,
    Settings { section: SettingsSection },
}

impl Route {
    /// The screen's title in the header, in French.
    pub fn title(&self) -> String {
        match self {
            Route::Login => "Connexion".into(),
            Route::Onboarding => "Bienvenue".into(),
            Route::Home => "Accueil".into(),
            Route::Search { .. } => "Recherche".into(),
            Route::Artist { .. } => "Artiste".into(),
            Route::Album { .. } => "Album".into(),
            Route::Playlist { .. } => "Playlist".into(),
            Route::Library { tab } => format!("Bibliothèque · {}", tab.title()),
            Route::Podcasts => "Podcasts".into(),
            Route::Show { .. } => "Émission".into(),
            Route::Releases => "Nouveautés".into(),
            Route::Recap { .. } => "Récap".into(),
            Route::History => "Historique".into(),
            Route::Genres => "Genres".into(),
            Route::Credits { kind, .. } => match kind {
                CreditKind::Track => "Crédits du titre".into(),
                CreditKind::Person => "Crédits".into(),
            },
            Route::Import { .. } => "Importer".into(),
            Route::Party => "Soirée d'écoute".into(),
            Route::Remote => "Contrôle distant".into(),
            Route::NowPlaying => "Lecture en cours".into(),
            Route::Queue => "File d'attente".into(),
            Route::Viz => "Visualiseur".into(),
            Route::Settings { section } => format!("Réglages · {}", section.title()),
        }
    }

    /// The key the scroll memory is stored under: the route AS a page, not the
    /// query that opened it, so typing a search and coming back restores the
    /// list at the same offset.
    pub fn scroll_key(&self) -> String {
        match self {
            Route::Search { .. } => "search".into(),
            Route::Settings { section } => format!("settings/{}", section.title()),
            Route::Library { tab } => format!("library/{}", tab.title()),
            Route::Recap { .. } => "recap".into(),
            other => other.title(),
        }
    }

    /// The nav item that owns this route, for the sidebar's active state.
    pub fn section(&self) -> Option<Section> {
        match self {
            Route::Home => Some(Section::Home),
            Route::Search { .. } => Some(Section::Search),
            Route::Library { .. }
            | Route::Playlist { .. }
            | Route::Album { .. }
            | Route::Artist { .. }
            | Route::Credits { .. }
            | Route::Import { .. } => Some(Section::Library),
            Route::Podcasts | Route::Show { .. } => Some(Section::Podcasts),
            Route::Releases | Route::Recap { .. } | Route::History | Route::Genres => {
                Some(Section::Releases)
            }
            Route::NowPlaying | Route::Queue | Route::Viz => Some(Section::NowPlaying),
            Route::Settings { .. } => Some(Section::Settings),
            Route::Party | Route::Remote | Route::Onboarding | Route::Login => None,
        }
    }
}

/// The five entries of the sidebar (plus réglages in the footer).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Home,
    Search,
    Library,
    Podcasts,
    Releases,
    NowPlaying,
    Settings,
}

impl Section {
    pub fn title(self) -> &'static str {
        match self {
            Section::Home => "Accueil",
            Section::Search => "Recherche",
            Section::Library => "Bibliothèque",
            Section::Podcasts => "Podcasts",
            Section::Releases => "Nouveautés",
            Section::NowPlaying => "Lecture",
            Section::Settings => "Réglages",
        }
    }

    pub fn route(self) -> Route {
        match self {
            Section::Home => Route::Home,
            Section::Search => Route::Search {
                query: String::new(),
            },
            Section::Library => Route::Library {
                tab: LibraryTab::default(),
            },
            Section::Podcasts => Route::Podcasts,
            Section::Releases => Route::Releases,
            Section::NowPlaying => Route::NowPlaying,
            Section::Settings => Route::Settings {
                section: SettingsSection::default(),
            },
        }
    }
}

/// One entry of the history.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NavEntry {
    pub route: Route,
}

/// The stack: back history, the current entry, and the forward history.
#[derive(Clone, Debug, Default)]
pub struct Nav {
    back: Vec<NavEntry>,
    current: Option<NavEntry>,
    forward: Vec<NavEntry>,
    scroll: HashMap<String, f32>,
    version: u64,
}

impl Nav {
    pub fn new(route: Route) -> Self {
        Self {
            current: Some(NavEntry { route }),
            ..Self::default()
        }
    }

    pub fn current(&self) -> Option<&Route> {
        self.current.as_ref().map(|entry| &entry.route)
    }

    /// Bumped on every change; the root view compares it to know when to build
    /// the screen for a new route.
    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn can_go_back(&self) -> bool {
        !self.back.is_empty()
    }

    pub fn can_go_forward(&self) -> bool {
        !self.forward.is_empty()
    }

    /// Go to `route`, remembering the current one. A push always clears the
    /// forward stack: the browser rule, and the one a person expects.
    pub fn push(&mut self, route: Route) {
        if self
            .current
            .as_ref()
            .is_some_and(|entry| entry.route == route)
        {
            return;
        }
        if let Some(entry) = self.current.take() {
            self.back.push(entry);
        }
        self.forward.clear();
        self.current = Some(NavEntry { route });
        self.version += 1;
    }

    /// Go to `route` in place (a query typed into the search field, a settings
    /// tab clicked): no history entry for every keystroke.
    pub fn replace(&mut self, route: Route) {
        match self.current.as_mut() {
            Some(entry) if entry.route == route => {}
            Some(entry) => {
                entry.route = route;
                self.version += 1;
            }
            None => {
                self.current = Some(NavEntry { route });
                self.version += 1;
            }
        }
    }

    pub fn back(&mut self) -> Option<&Route> {
        let previous = self.back.pop()?;
        if let Some(entry) = self.current.take() {
            self.forward.push(entry);
        }
        self.current = Some(previous);
        self.version += 1;
        self.current()
    }

    pub fn forward(&mut self) -> Option<&Route> {
        let next = self.forward.pop()?;
        if let Some(entry) = self.current.take() {
            self.back.push(entry);
        }
        self.current = Some(next);
        self.version += 1;
        self.current()
    }

    /// After a login/logout the whole history is meaningless: drop it.
    pub fn reset(&mut self, route: Route) {
        self.back.clear();
        self.forward.clear();
        self.current = Some(NavEntry { route });
        self.version += 1;
    }

    pub fn save_scroll(&mut self, route: &Route, offset: f32) {
        self.scroll.insert(route.scroll_key(), offset);
    }

    pub fn scroll(&self, route: &Route) -> f32 {
        self.scroll.get(&route.scroll_key()).copied().unwrap_or(0.0)
    }

    /// The breadcrumb of history routes, oldest first. What a "back" button's
    /// tooltip names, and what a test asserts against.
    pub fn history(&self) -> Vec<&Route> {
        self.back
            .iter()
            .map(|entry| &entry.route)
            .chain(self.current.iter().map(|entry| &entry.route))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_back_forward_keeps_the_history_a_person_expects() {
        let mut nav = Nav::new(Route::Home);
        nav.push(Route::Artist { id: "1".into() });
        nav.push(Route::Album { id: "2".into() });
        assert_eq!(nav.current(), Some(&Route::Album { id: "2".into() }));
        assert!(nav.can_go_back());

        nav.back();
        assert_eq!(nav.current(), Some(&Route::Artist { id: "1".into() }));
        nav.back();
        assert_eq!(nav.current(), Some(&Route::Home));
        assert!(!nav.can_go_back());
        assert_eq!(nav.back(), None, "an empty history keeps the current page");

        nav.forward();
        assert_eq!(nav.current(), Some(&Route::Artist { id: "1".into() }));
    }

    #[test]
    fn a_new_push_clears_the_forward_stack() {
        let mut nav = Nav::new(Route::Home);
        nav.push(Route::Podcasts);
        nav.back();
        assert!(nav.can_go_forward());
        nav.push(Route::Genres);
        assert!(!nav.can_go_forward());
        assert_eq!(nav.history(), [&Route::Home, &Route::Genres]);
    }

    #[test]
    fn replace_does_not_grow_the_history() {
        let mut nav = Nav::new(Route::Search { query: "ka".into() });
        let version = nav.version();
        nav.replace(Route::Search {
            query: "kaya".into(),
        });
        assert_eq!(nav.history().len(), 1);
        assert_eq!(nav.version(), version + 1);
        assert_eq!(
            nav.current(),
            Some(&Route::Search {
                query: "kaya".into()
            })
        );
        // Replacing with the same route is not a change: no redraw, no version.
        let version = nav.version();
        nav.replace(Route::Search {
            query: "kaya".into(),
        });
        assert_eq!(nav.version(), version);
    }

    #[test]
    fn scroll_positions_are_per_page_not_per_query() {
        let mut nav = Nav::new(Route::Search { query: "a".into() });
        nav.save_scroll(&Route::Search { query: "a".into() }, 812.5);
        assert_eq!(nav.scroll(&Route::Search { query: "b".into() }), 812.5);
        nav.save_scroll(
            &Route::Library {
                tab: LibraryTab::Favorites,
            },
            42.0,
        );
        assert_eq!(nav.scroll(&Route::Search { query: "c".into() }), 812.5);
        assert_eq!(nav.scroll(&Route::Home), 0.0);
    }

    #[test]
    fn reset_drops_the_history_after_a_login_or_a_logout() {
        let mut nav = Nav::new(Route::Home);
        nav.push(Route::Settings {
            section: SettingsSection::Playback,
        });
        nav.reset(Route::Login);
        assert!(!nav.can_go_back());
        assert!(!nav.can_go_forward());
        assert_eq!(nav.current(), Some(&Route::Login));
    }

    #[test]
    fn every_section_route_is_owned_by_that_section() {
        for section in [
            Section::Home,
            Section::Search,
            Section::Library,
            Section::Podcasts,
            Section::Releases,
            Section::NowPlaying,
            Section::Settings,
        ] {
            assert_eq!(section.route().section(), Some(section));
        }
    }
}

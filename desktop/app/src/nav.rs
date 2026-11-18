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
    /// The token [`Route::spec`] uses, spelled as the tab's title without its
    /// accents and case (`favoris` → `favorites`).
    pub fn slug(self) -> &'static str {
        match self {
            LibraryTab::Playlists => "playlists",
            LibraryTab::Favorites => "favorites",
            LibraryTab::Artists => "artists",
            LibraryTab::Albums => "albums",
            LibraryTab::Uploads => "uploads",
            LibraryTab::Dislikes => "dislikes",
            LibraryTab::Unavailable => "unavailable",
            LibraryTab::Folders => "folders",
        }
    }

    pub fn from_slug(slug: &str) -> Option<Self> {
        Some(match slug {
            "playlists" => LibraryTab::Playlists,
            "favorites" => LibraryTab::Favorites,
            "artists" => LibraryTab::Artists,
            "albums" => LibraryTab::Albums,
            "uploads" => LibraryTab::Uploads,
            "dislikes" => LibraryTab::Dislikes,
            "unavailable" => LibraryTab::Unavailable,
            "folders" => LibraryTab::Folders,
            _ => return None,
        })
    }

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
    Lan,
    Help,
    Shortcuts,
}

impl SettingsSection {
    /// The token [`Route::spec`] uses, in the title's own words, unhyphenated
    /// (`transcription-locale`).
    pub fn slug(self) -> &'static str {
        match self {
            SettingsSection::Playback => "lecture-et-son",
            SettingsSection::Appearance => "apparence",
            SettingsSection::Viz => "visualiseur",
            SettingsSection::Downloads => "telechargements",
            SettingsSection::Discovery => "decouverte",
            SettingsSection::Sources => "sources",
            SettingsSection::Account => "compte",
            SettingsSection::Users => "utilisateurs",
            SettingsSection::Server => "serveur",
            SettingsSection::Transcribe => "transcription-locale",
            SettingsSection::Lan => "appareils-a-proximite",
            SettingsSection::Help => "assistance",
            SettingsSection::Shortcuts => "raccourcis",
        }
    }

    pub fn from_slug(slug: &str) -> Option<Self> {
        Some(match slug {
            "lecture-et-son" => SettingsSection::Playback,
            "apparence" => SettingsSection::Appearance,
            "visualiseur" => SettingsSection::Viz,
            "telechargements" => SettingsSection::Downloads,
            "decouverte" => SettingsSection::Discovery,
            "sources" => SettingsSection::Sources,
            "compte" => SettingsSection::Account,
            "utilisateurs" => SettingsSection::Users,
            "serveur" => SettingsSection::Server,
            "transcription-locale" => SettingsSection::Transcribe,
            "appareils-a-proximite" => SettingsSection::Lan,
            "assistance" => SettingsSection::Help,
            "raccourcis" => SettingsSection::Shortcuts,
            _ => return None,
        })
    }

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
            SettingsSection::Lan => "Appareils à proximité",
            SettingsSection::Help => "Assistance",
            SettingsSection::Shortcuts => "Raccourcis",
        }
    }

    /// The sections a non-admin sees, in the order the SPA lists them.
    pub const USER: [SettingsSection; 11] = [
        SettingsSection::Playback,
        SettingsSection::Appearance,
        SettingsSection::Viz,
        SettingsSection::Downloads,
        SettingsSection::Discovery,
        SettingsSection::Sources,
        // The SPA's own place for its « Appareils et amis » category: after the
        // sources, before the account (LN8, `routes/Settings.svelte#SECTIONS`).
        SettingsSection::Lan,
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

/// How a person reaches a route.
///
/// D17 audited every screen of the app against one question: can it be OPENED
/// without knowing that it exists? A route nobody can reach is a screen the app
/// does not have, and the shell's own table (`screens::all_routes`) would still
/// build it — so the answer is written here, next to the route, and pinned by a
/// test. The SPA's inventory is the reference: every entry of `LibraryNav` and
/// every tab of `Settings` is reachable by a click, and everything else by the
/// sidebar or a shortcut.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    /// The app lands there by itself: the login screen, onboarding after a
    /// first login, a failure screen's own « Réessayer ».
    Automatic,
    /// A row of the sidebar.
    Sidebar,
    /// A keystroke (the spelling the `?` overlay prints, `keymap::SHORTCUTS`).
    Shortcut(&'static str),
    /// A link on another screen (a row, a card, a button).
    Link(&'static str),
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

    /// The route as one token, and back: `library:favorites`,
    /// `settings:transcription-locale`, `artist:123`. D17's journey bench opens
    /// each screen with `--route <token>` instead of clicking blind, so every
    /// capture and every request count is the same screen on every run; the
    /// round trip is a test, because the bench and the app must not drift.
    pub fn spec(&self) -> String {
        match self {
            Route::Login => "login".into(),
            Route::Onboarding => "onboarding".into(),
            Route::Home => "home".into(),
            Route::Search { query } => {
                if query.is_empty() {
                    "search".into()
                } else {
                    format!("search:{query}")
                }
            }
            Route::Artist { id } => format!("artist:{id}"),
            Route::Album { id } => format!("album:{id}"),
            Route::Playlist { id } => format!("playlist:{id}"),
            Route::Library { tab } => format!("library:{}", tab.slug()),
            Route::Podcasts => "podcasts".into(),
            Route::Show { id } => format!("show:{id}"),
            Route::Releases => "releases".into(),
            Route::Recap { year } => match year {
                Some(year) => format!("recap:{year}"),
                None => "recap".into(),
            },
            Route::History => "history".into(),
            Route::Genres => "genres".into(),
            Route::Credits { kind, id } => format!(
                "credits:{}:{id}",
                match kind {
                    CreditKind::Track => "track",
                    CreditKind::Person => "person",
                }
            ),
            Route::Import { url } => match url {
                Some(url) => format!("import:{url}"),
                None => "import".into(),
            },
            Route::Party => "party".into(),
            Route::Remote => "remote".into(),
            Route::NowPlaying => "now-playing".into(),
            Route::Queue => "queue".into(),
            Route::Viz => "viz".into(),
            Route::Settings { section } => format!("settings:{}", section.slug()),
        }
    }

    /// Parse a [`Route::spec`]. `None` for anything that is not one, so a
    /// mistyped `--route` is an error the app prints, never a silent Home.
    pub fn parse_spec(spec: &str) -> Option<Route> {
        let (head, rest) = match spec.split_once(':') {
            Some((head, rest)) => (head, Some(rest)),
            None => (spec, None),
        };
        let required = || rest.filter(|value| !value.is_empty());
        Some(match head {
            "login" => Route::Login,
            "onboarding" => Route::Onboarding,
            "home" => Route::Home,
            "search" => Route::Search {
                query: rest.unwrap_or("").to_owned(),
            },
            "artist" => Route::Artist {
                id: required()?.to_owned(),
            },
            "album" => Route::Album {
                id: required()?.to_owned(),
            },
            "playlist" => Route::Playlist {
                id: required()?.to_owned(),
            },
            "library" => Route::Library {
                tab: rest.and_then(LibraryTab::from_slug).unwrap_or_default(),
            },
            "podcasts" => Route::Podcasts,
            "show" => Route::Show {
                id: required()?.to_owned(),
            },
            "releases" => Route::Releases,
            "recap" => Route::Recap {
                year: rest.and_then(|year| year.parse().ok()),
            },
            "history" => Route::History,
            "genres" => Route::Genres,
            "credits" => {
                let (kind, id) = rest?.split_once(':')?;
                Route::Credits {
                    kind: match kind {
                        "track" => CreditKind::Track,
                        "person" => CreditKind::Person,
                        _ => return None,
                    },
                    id: id.to_owned(),
                }
            }
            "import" => Route::Import {
                url: rest.map(str::to_owned),
            },
            "party" => Route::Party,
            "remote" => Route::Remote,
            "now-playing" => Route::NowPlaying,
            "queue" => Route::Queue,
            "viz" => Route::Viz,
            "settings" => Route::Settings {
                section: rest
                    .and_then(SettingsSection::from_slug)
                    .unwrap_or_default(),
            },
            _ => return None,
        })
    }

    /// The optional feature this screen belongs to, when one server may lack
    /// it. `None` for the screens every server from the app's first version
    /// has (`nz_api::Feature`).
    pub fn feature(&self) -> Option<crate::state::Feature> {
        match self {
            Route::Party => Some(crate::state::Feature::Party),
            Route::Remote => Some(crate::state::Feature::Remote),
            Route::Genres => Some(crate::state::Feature::Genres),
            Route::Settings {
                section: SettingsSection::Transcribe,
            } => Some(crate::state::Feature::Transcription),
            // « Appareils à proximité » is the LAN's Réglages section: an older
            // server has no `/api/lan/*`, and the route says so instead of
            // painting a section that could never work (D17).
            Route::Settings {
                section: SettingsSection::Lan,
            } => Some(crate::state::Feature::Lan),
            // NOT `Library { tab: Folders }`: the Bibliothèque exists on an
            // older server, only its folder tab does not, and replacing the
            // whole screen would hide the playlists with it. The library's tab
            // strip is where that one would be hidden.
            _ => None,
        }
    }

    /// The way in: how a person who has never seen this screen can open it.
    ///
    /// A screen that only answers a short-lived internal link (an action, a
    /// toast, a push from a state machine) is not *reachable*: D17 found the
    /// full-screen queue in exactly that hole — `Route::Queue` existed, the
    /// side panel drew the same screen, and nothing in the app ever pushed the
    /// route. `g q` is that way in now, and this table is what keeps the next
    /// route from repeating the mistake.
    pub fn access(&self) -> Access {
        match self {
            Route::Login | Route::Onboarding => Access::Automatic,
            Route::Home
            | Route::Search { .. }
            | Route::Library { .. }
            | Route::Podcasts
            | Route::Releases
            | Route::NowPlaying
            | Route::Settings { .. } => Access::Sidebar,
            Route::Artist { .. } => Access::Link("un titre, un album, une recherche"),
            Route::Album { .. } => Access::Link("un titre, une recherche, la Bibliothèque"),
            Route::Playlist { .. } => Access::Link("la Bibliothèque"),
            Route::Show { .. } => Access::Link("les Podcasts"),
            Route::Recap { .. } => Access::Link("Nouveautés, ou l'Accueil"),
            Route::History => Access::Link("Nouveautés"),
            Route::Genres => Access::Shortcut("g e"),
            Route::Credits { .. } => Access::Link("le menu contextuel d'un titre"),
            Route::Import { .. } => Access::Link("la Bibliothèque"),
            Route::Party => Access::Sidebar,
            Route::Remote => Access::Sidebar,
            Route::Queue => Access::Shortcut("g q"),
            Route::Viz => Access::Shortcut("v"),
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

    /// D17: the bench opens each screen with `--route <spec>`, so the token
    /// and the route must round-trip for every one of them — an unmatched spec
    /// is a screen the bench would silently photograph as Accueil.
    #[test]
    fn every_route_round_trips_through_its_bench_token() {
        for route in crate::screens::all_routes() {
            let spec = route.spec();
            assert_eq!(
                Route::parse_spec(&spec),
                Some(route.clone()),
                "{spec} does not parse back"
            );
        }
        assert_eq!(
            Route::parse_spec("library:favorites"),
            Some(Route::Library {
                tab: LibraryTab::Favorites
            })
        );
        assert_eq!(Route::parse_spec("nope"), None);
        assert_eq!(Route::parse_spec("artist:"), None);
        assert_eq!(Route::parse_spec("credits:album:1"), None);
    }

    /// D17: only the screens an older server may lack name a feature; a screen
    /// that names one is hidden with it.
    #[test]
    fn only_the_optional_screens_name_a_feature() {
        use crate::state::Feature;
        assert_eq!(Route::Party.feature(), Some(Feature::Party));
        assert_eq!(Route::Remote.feature(), Some(Feature::Remote));
        assert_eq!(Route::Genres.feature(), Some(Feature::Genres));
        assert_eq!(
            Route::Settings {
                section: SettingsSection::Transcribe
            }
            .feature(),
            Some(Feature::Transcription)
        );
        for route in [
            Route::Home,
            Route::Search {
                query: String::new(),
            },
            Route::NowPlaying,
            Route::Queue,
            Route::Viz,
            Route::Library {
                tab: LibraryTab::Folders,
            },
        ] {
            assert_eq!(route.feature(), None, "{route:?}");
        }
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

use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use image::DynamicImage;
use ratatui_image::{picker::Picker, protocol::StatefulProtocol};
use crate::{
    api::{ApiClient, models::*},
    audio,
    auth,
    config::{Config, StreamQuality},
    socket::SocketClient,
    tui::{
        self,
        events::{AppEvent, EventHandler},
        layout::AppLayout,
        views::View,
    },
};

#[derive(Debug, Clone, PartialEq)]
pub enum ConnectionStatus {
    Connected,
    Disconnected,
    Connecting,
}

/// Audio commands queued by event handlers and processed in the main loop.
pub enum PendingAudioCommand {
    /// Start the configured main stream URL.
    Start,
    /// Start a specific stream URL (e.g., song preview).
    StartUrl(String),
    Pause,
    Resume,
    SetVolume(f32),
}

#[derive(Debug, Clone)]
pub struct PaginatedState<T> {
    pub items: Vec<T>,
    pub page: u32,
    pub last_page: u32,
    pub selected: usize,
    pub loading: bool,
}

impl<T> Default for PaginatedState<T> {
    fn default() -> Self {
        PaginatedState {
            items: Vec::new(),
            page: 0,
            last_page: 0,
            selected: 0,
            loading: false,
        }
    }
}

/// State for the song detail popup (opened from History / Charts).
#[derive(Debug, Clone)]
pub struct SongDetailState {
    pub song: Song,
    /// Unix timestamp of when this song was played (from history entry).
    pub played_at: Option<i64>,
    pub is_favorited: bool,
    pub favorite_id: Option<u64>,
    /// True while favorite status is being fetched.
    pub loading_favorite: bool,
    /// True while the preview is playing.
    pub is_previewing: bool,
}

impl SongDetailState {
    pub fn new(song: Song, played_at: Option<i64>) -> Self {
        SongDetailState {
            song,
            played_at,
            is_favorited: false,
            favorite_id: None,
            loading_favorite: true,
            is_previewing: false,
        }
    }
}

pub struct AppState {
    pub view: View,
    pub now_playing: Option<StatusResource>,
    /// When the last status update was received (for client-side position tracking).
    pub status_received_at: Option<Instant>,
    pub is_playing: bool,
    pub is_authenticated: bool,
    pub volume: f32,
    pub connection: ConnectionStatus,
    pub history: PaginatedState<HistoryEntry>,
    pub favorites: PaginatedState<FavoriteEntry>,
    pub charts: PaginatedState<RatingEntry>,
    pub chart_range: RatingRange,
    pub news: PaginatedState<NewsItem>,
    pub user: Option<User>,
    pub user_stats: Option<UserStats>,
    pub notification: Option<(String, Instant)>,
    pub show_help: bool,
    pub error: Option<String>,
    pub login_username: String,
    pub login_password: String,
    pub login_focus_password: bool,
    pub login_error: Option<String>,
    pub show_song_detail: Option<SongDetailState>,
    pub show_delete_confirm: bool,
    pub show_logout_confirm: bool,
    /// Audio command to execute on the next main-loop iteration.
    pub pending_audio: Option<PendingAudioCommand>,
    /// Artwork URL of the currently displayed song (used to detect song changes).
    pub artwork_url: Option<String>,
}

impl AppState {
    pub fn new(config: &Config) -> Self {
        AppState {
            view: View::Login,
            now_playing: None,
            is_playing: false,
            is_authenticated: false,
            volume: config.volume,
            connection: ConnectionStatus::Connecting,
            history: Default::default(),
            favorites: Default::default(),
            charts: Default::default(),
            chart_range: RatingRange::Overtime,
            news: Default::default(),
            user: None,
            user_stats: None,
            status_received_at: None,
            notification: None,
            show_help: false,
            error: None,
            login_username: String::new(),
            login_password: String::new(),
            login_focus_password: false,
            login_error: None,
            show_song_detail: None,
            show_delete_confirm: false,
            show_logout_confirm: false,
            pending_audio: None,
            artwork_url: None,
        }
    }

    pub fn notify(&mut self, msg: impl Into<String>) {
        self.notification = Some((msg.into(), Instant::now()));
    }

    pub fn clear_expired_notification(&mut self) {
        if let Some((_, time)) = &self.notification {
            if time.elapsed() > Duration::from_secs(3) {
                self.notification = None;
            }
        }
    }
}

pub async fn run(config: Config, mut api: ApiClient) -> anyhow::Result<()> {
    let mut terminal = tui::setup_terminal()?;

    // Set up image picker after entering alternate screen but before event loop.
    // from_query_stdio() queries the terminal for protocol support and font size.
    let picker = Picker::from_query_stdio().unwrap_or_else(|_| Picker::halfblocks());
    tracing::info!("Image protocol: {:?}", picker.protocol_type());

    // Artwork state: current artwork protocol + channel for async fetch results
    let mut artwork: Option<StatefulProtocol> = None;
    let mut current_artwork_url: Option<String> = None;
    let (artwork_img_tx, mut artwork_img_rx) = mpsc::channel::<DynamicImage>(2);

    // Audio player (non-fatal — app functions without audio output device)
    let mut player: Option<audio::Player> = match audio::Player::new() {
        Ok(p) => {
            tracing::info!("Audio player initialized");
            Some(p)
        }
        Err(e) => {
            tracing::warn!("Audio player init failed (no audio output?): {}", e);
            None
        }
    };

    // Audio error channel — the audio thread reports unrecoverable failures (e.g. an
    // undecodable codec) here so the UI can surface them instead of silently retrying.
    let (audio_error_tx, audio_error_rx) = mpsc::channel::<String>(8);
    if let Some(p) = &mut player {
        p.set_error_sender(audio_error_tx.clone());
    }
    let _audio_error_tx = audio_error_tx; // keep alive so channel never auto-closes

    // Socket client (non-fatal)
    let socket = SocketClient::connect().await.ok();
    let (dummy_tx, _) = tokio::sync::broadcast::channel::<crate::socket::SocketEvent>(1);
    let socket_rx = match &socket {
        Some(s) => {
            tracing::info!("Socket.io client connected");
            s.subscribe()
        }
        None => {
            tracing::warn!("Socket unavailable — running without real-time updates");
            dummy_tx.subscribe()
        }
    };

    let mut event_handler = EventHandler::new(socket_rx, audio_error_rx);
    let mut state = AppState::new(&config);

    // Resolve the stream we can actually decode. If the saved preference is a codec
    // this build can't decode yet (Opus/HLS until Phase 1), fall back to MP3 for this
    // session without overwriting the stored preference, and tell the user.
    let stream_quality = if config.stream_quality.is_supported() {
        config.stream_quality.clone()
    } else {
        tracing::warn!(
            "Stream quality {:?} ({}) is not decodable in this build yet; falling back to MP3",
            config.stream_quality,
            config.stream_quality.label()
        );
        state.notify(format!(
            "{} not supported yet — playing MP3 128k instead",
            config.stream_quality.label()
        ));
        StreamQuality::Mp3
    };
    let stream_url = stream_quality.stream_url().to_string();
    tracing::info!("Active stream: {} ({})", stream_quality.label(), stream_url);

    // If a saved token exists, skip login and go straight to NowPlaying
    if api.is_authenticated() {
        tracing::info!("Saved token found, skipping login");
        state.is_authenticated = true;
        state.view = View::NowPlaying;
        match api.get_status().await {
            Ok(status) => {
                tracing::info!("Status loaded: {} — {}", status.song.artist, status.song.title);
                state.artwork_url = status.song.artwork_sm_src.clone()
                    .or_else(|| status.song.artwork_src.clone());
                state.status_received_at = Some(Instant::now());
                state.now_playing = Some(status);
            }
            Err(e) => tracing::warn!("Failed to load initial status: {}", e),
        }
        state.pending_audio = Some(PendingAudioCommand::Start);
    }

    // Background pre-fetch channels: history and charts data load in the background
    // so the first navigation to those views is instant.
    let (history_prefetch_tx, mut history_prefetch_rx) = mpsc::channel::<Paginated<HistoryEntry>>(2);
    let (charts_prefetch_tx, mut charts_prefetch_rx) = mpsc::channel::<Paginated<RatingEntry>>(2);
    let mut last_song_id_for_prefetch: Option<String> = None;
    // Favorite status for the open popup: (is_favorited, favorite_id)
    let (favorite_status_tx, mut favorite_status_rx) = mpsc::channel::<(bool, Option<u64>)>(2);
    // Popup artwork
    let (popup_art_tx, mut popup_art_rx) = mpsc::channel::<DynamicImage>(2);
    let mut popup_artwork: Option<StatefulProtocol> = None;
    let mut popup_artwork_url: Option<String> = None;

    // Kick off background pre-fetch of history and charts immediately.
    {
        let api2 = api.clone();
        let tx = history_prefetch_tx.clone();
        tokio::spawn(async move {
            if let Ok(h) = api2.get_history(1).await {
                let _ = tx.send(h).await;
            }
        });
    }
    {
        let api2 = api.clone();
        let tx = charts_prefetch_tx.clone();
        tokio::spawn(async move {
            if let Ok(c) = api2.get_ratings(RatingRange::Overtime, 1).await {
                let _ = tx.send(c).await;
            }
        });
    }

    // Main loop
    let mut prev_show_help = false;
    let mut prev_show_popup = false;
    let mut prev_popup_song_id: Option<String> = None;
    loop {
        // Check if the song changed and we need to fetch new artwork
        if state.artwork_url.as_deref() != current_artwork_url.as_deref() {
            current_artwork_url = state.artwork_url.clone();
            artwork = None; // Clear old artwork while fetching new
            if let Some(url) = state.artwork_url.clone() {
                let tx = artwork_img_tx.clone();
                tokio::spawn(async move {
                    match reqwest::get(&url).await {
                        Ok(resp) => match resp.bytes().await {
                            Ok(bytes) => match image::load_from_memory(&bytes) {
                                Ok(img) => { let _ = tx.send(img).await; }
                                Err(e) => tracing::warn!("Artwork decode failed: {}", e),
                            },
                            Err(e) => tracing::warn!("Artwork fetch body failed: {}", e),
                        },
                        Err(e) => tracing::warn!("Artwork fetch failed: {}", e),
                    }
                });
            }
        }

        // Receive decoded artwork if ready
        if let Ok(img) = artwork_img_rx.try_recv() {
            tracing::debug!("Artwork decoded, creating protocol");
            artwork = Some(picker.new_resize_protocol(img));
        }

        // Receive pre-fetched history (only if not yet loaded by user navigation)
        if let Ok(h) = history_prefetch_rx.try_recv() {
            if state.history.items.is_empty() {
                tracing::debug!("Pre-fetched history: {} items", h.data.len());
                state.history.items = h.data;
                state.history.last_page = h.meta.last_page;
                state.history.page = 1;
            }
        }
        // Receive pre-fetched charts
        if let Ok(c) = charts_prefetch_rx.try_recv() {
            if state.charts.items.is_empty() {
                tracing::debug!("Pre-fetched charts: {} items", c.data.len());
                state.charts.items = c.data;
                state.charts.last_page = c.meta.last_page;
                state.charts.page = 1;
            }
        }

        // Re-fetch history when song changes (so it's fresh when user navigates there)
        let current_song_id = state.now_playing.as_ref().map(|np| np.song.id_str());
        if current_song_id != last_song_id_for_prefetch && current_song_id.is_some() {
            last_song_id_for_prefetch = current_song_id;
            let api2 = api.clone();
            let tx = history_prefetch_tx.clone();
            tokio::spawn(async move {
                if let Ok(h) = api2.get_history(1).await {
                    let _ = tx.send(h).await;
                }
            });
        }

        // Popup: receive favorite status
        if let Ok((is_fav, fav_id)) = favorite_status_rx.try_recv() {
            if let Some(detail) = &mut state.show_song_detail {
                detail.is_favorited = is_fav;
                detail.favorite_id = fav_id;
                detail.loading_favorite = false;
            }
        }

        // Popup: spawn favorite status fetch when a new popup is opened
        let popup_id = state.show_song_detail.as_ref().map(|d| d.song.id_str());
        if popup_id != prev_popup_song_id {
            prev_popup_song_id = popup_id.clone();
            if let Some(id) = popup_id {
                let api2 = api.clone();
                let tx = favorite_status_tx.clone();
                tokio::spawn(async move {
                    match api2.get_song(&id).await {
                        Ok(sr) => { let _ = tx.send((sr.is_favorited, sr.favorite_id)).await; }
                        Err(_) => { let _ = tx.send((false, None)).await; }
                    }
                });
            }
        }

        // Popup: artwork tracking
        let popup_url = state.show_song_detail.as_ref()
            .and_then(|d| d.song.artwork_src.clone().or_else(|| d.song.artwork_sm_src.clone()));
        if popup_url != popup_artwork_url {
            popup_artwork_url = popup_url.clone();
            popup_artwork = None;
            if let Some(url) = popup_url {
                let tx = popup_art_tx.clone();
                tokio::spawn(async move {
                    match reqwest::get(&url).await {
                        Ok(resp) => match resp.bytes().await {
                            Ok(bytes) => match image::load_from_memory(&bytes) {
                                Ok(img) => { let _ = tx.send(img).await; }
                                Err(e) => tracing::warn!("Popup artwork decode failed: {}", e),
                            },
                            Err(e) => tracing::warn!("Popup artwork fetch body failed: {}", e),
                        },
                        Err(e) => tracing::warn!("Popup artwork fetch failed: {}", e),
                    }
                });
            }
        }
        if let Ok(img) = popup_art_rx.try_recv() {
            popup_artwork = Some(picker.new_resize_protocol(img));
        }

        // Process pending audio command from the previous iteration
        if let Some(cmd) = state.pending_audio.take() {
            match cmd {
                PendingAudioCommand::Start => {
                    if let Some(ref mut p) = player {
                        // Always stop first — start_live_stream calls stop_inner() internally,
                        // so this correctly replaces any currently-playing preview.
                        tracing::info!("Starting stream: {}", stream_url);
                        match p.start_live_stream(stream_url.clone()) {
                            Ok(()) => {
                                state.is_playing = true;
                                tracing::info!("Audio playback started");
                            }
                            Err(e) => {
                                tracing::error!("Player start error: {}", e);
                                state.notify(format!("Audio error: {}", e));
                            }
                        }
                    } else {
                        tracing::warn!("No audio output device — stream not started");
                        state.notify("No audio output device available");
                    }
                }
                PendingAudioCommand::StartUrl(url) => {
                    if let Some(ref mut p) = player {
                        tracing::info!("Starting stream URL: {}", url);
                        match p.start_stream(url) {
                            Ok(()) => { state.is_playing = true; }
                            Err(e) => {
                                tracing::error!("Player start_url error: {}", e);
                                state.notify(format!("Audio error: {}", e));
                            }
                        }
                    }
                }
                PendingAudioCommand::Pause => {
                    if let Some(ref mut p) = player {
                        p.pause();
                        state.is_playing = false;
                        tracing::info!("Audio paused");
                    }
                }
                PendingAudioCommand::Resume => {
                    if let Some(ref mut p) = player {
                        p.resume();
                        state.is_playing = true;
                        tracing::info!("Audio resumed");
                    }
                }
                PendingAudioCommand::SetVolume(v) => {
                    if let Some(ref mut p) = player {
                        p.set_volume(v);
                        tracing::debug!("Volume set to {:.0}%", v * 100.0);
                    }
                }
            }
        }

        // Force full terminal clear when any overlay is dismissed to erase stale cells.
        let show_popup = state.show_song_detail.is_some();
        if (prev_show_help && !state.show_help) || (prev_show_popup && !show_popup) {
            terminal.clear()?;
        }
        prev_show_help = state.show_help;
        prev_show_popup = show_popup;

        // Render
        terminal.draw(|frame| {
            render(frame, &state, &mut artwork, &mut popup_artwork);
        })?;

        // Wait for next event
        let event = event_handler.next().await;

        match handle_event(event, &mut state, &mut api).await {
            EventAction::Continue => {}
            EventAction::Quit => break,
        }

        state.clear_expired_notification();
    }

    // Clean shutdown
    if let Some(ref mut p) = player {
        p.stop();
    }
    tui::restore_terminal(&mut terminal)?;
    Ok(())
}

enum EventAction {
    Continue,
    Quit,
}

async fn handle_event(event: AppEvent, state: &mut AppState, api: &mut ApiClient) -> EventAction {
    use crossterm::event::KeyCode;

    match event {
        AppEvent::Quit => return EventAction::Quit,

        AppEvent::StatusUpdate(s) => {
            tracing::debug!("Socket status update: {} — {}", s.song.artist, s.song.title);
            // Detect song change → invalidate history so it refreshes next view
            let song_changed = state.now_playing.as_ref()
                .map(|np| np.song.id_str() != s.song.id_str())
                .unwrap_or(true);
            if song_changed {
                state.history.items.clear();
                state.history.page = 0;
            }
            // Track artwork URL for change detection in main loop
            state.artwork_url = s.song.artwork_sm_src.clone()
                .or_else(|| s.song.artwork_src.clone());
            state.status_received_at = Some(Instant::now());
            state.now_playing = Some(s);
            state.connection = ConnectionStatus::Connected;
        }
        AppEvent::ListenersUpdate(n) => {
            if let Some(np) = &mut state.now_playing {
                np.listeners = n;
            }
        }
        AppEvent::ReactionsUpdate(n) => {
            if let Some(np) = &mut state.now_playing {
                np.song.reactions = n;
            }
        }
        AppEvent::AudioError(e) => {
            tracing::error!("Audio error: {}", e);
            state.error = Some(e);
            state.is_playing = false;
        }

        // Paste events: append to whichever login field is focused
        AppEvent::Paste(text) => {
            if state.view == View::Login {
                tracing::debug!("Paste event ({} chars)", text.len());
                if state.login_focus_password {
                    state.login_password.push_str(&text);
                } else {
                    state.login_username.push_str(&text);
                }
            }
        }

        AppEvent::Key(key) => {
            // Global keys — guarded so they don't fire during login
            match key.code {
                KeyCode::Char('q') if state.view != View::Login => return EventAction::Quit,
                KeyCode::Char('?') if state.view != View::Login => {
                    state.show_help = !state.show_help;
                }
                KeyCode::Char('1') if state.view != View::Login => {
                    state.view = View::NowPlaying;
                }
                KeyCode::Char('2') if state.view != View::Login => {
                    state.view = View::History;
                    if state.history.items.is_empty() {
                        tracing::info!("Loading history page 1");
                        match api.get_history(1).await {
                            Ok(h) => {
                                state.history.items = h.data;
                                state.history.last_page = h.meta.last_page;
                                state.history.page = 1;
                                tracing::info!("History loaded: {} items", state.history.items.len());
                            }
                            Err(e) => {
                                tracing::warn!("History load failed: {}", e);
                                state.notify(format!("Failed to load history: {}", e));
                            }
                        }
                    }
                }
                KeyCode::Char('3') if state.view != View::Login => {
                    state.view = View::Favorites;
                    if state.favorites.items.is_empty() && state.is_authenticated {
                        tracing::info!("Loading favorites page 1");
                        match api.get_favorites(1).await {
                            Ok(f) => {
                                state.favorites.items = f.data;
                                state.favorites.last_page = f.meta.last_page;
                                state.favorites.page = 1;
                            }
                            Err(e) => {
                                tracing::warn!("Favorites load failed: {}", e);
                            }
                        }
                    }
                }
                KeyCode::Char('4') if state.view != View::Login => {
                    state.view = View::Charts;
                    if state.charts.items.is_empty() {
                        tracing::info!("Loading charts ({:?})", state.chart_range.as_str());
                        match api.get_ratings(state.chart_range, 1).await {
                            Ok(c) => {
                                state.charts.items = c.data;
                                state.charts.last_page = c.meta.last_page;
                                state.charts.page = 1;
                            }
                            Err(e) => {
                                tracing::warn!("Charts load failed: {}", e);
                                state.notify(format!("Failed to load charts: {}", e));
                            }
                        }
                    }
                }
                KeyCode::Char('5') if state.view != View::Login => {
                    state.view = View::News;
                    if state.news.items.is_empty() {
                        tracing::info!("Loading news page 1");
                        match api.get_news(1).await {
                            Ok(n) => {
                                state.news.items = n.data;
                                state.news.last_page = n.meta.last_page;
                                state.news.page = 1;
                                tracing::info!("News loaded: {} items", state.news.items.len());
                            }
                            Err(e) => {
                                tracing::warn!("News load failed: {}", e);
                                state.notify(format!("Failed to load news: {}", e));
                            }
                        }
                    }
                }
                KeyCode::Char('6') if state.view != View::Login => {
                    state.view = View::Profile;
                    if state.user.is_none() && state.is_authenticated {
                        tracing::info!("Loading profile");
                        match api.get_me().await {
                            Ok(u) => { state.user = Some(u); }
                            Err(e) => tracing::error!("get_me failed: {}", e),
                        }
                        match api.get_my_stats().await {
                            Ok(s) => { state.user_stats = Some(s); }
                            Err(e) => tracing::error!("get_my_stats failed: {}", e),
                        }
                    }
                }
                _ => {
                    handle_view_key(key, state, api).await;
                }
            }
        }
        _ => {}
    }

    EventAction::Continue
}

async fn handle_view_key(
    key: crossterm::event::KeyEvent,
    state: &mut AppState,
    api: &mut ApiClient,
) {
    // Dismiss help overlay first
    if state.show_help {
        state.show_help = false;
        return;
    }

    // Song detail popup key handling
    if state.show_song_detail.is_some() {
        use crossterm::event::KeyCode;
        match key.code {
            // Close popup
            KeyCode::Esc | KeyCode::Char('q') => {
                if let Some(detail) = state.show_song_detail.take() {
                    // If we were previewing, restart the main stream
                    if detail.is_previewing {
                        state.pending_audio = Some(PendingAudioCommand::Start);
                    }
                }
            }
            // Toggle favorite
            KeyCode::Char('f') if state.is_authenticated => {
                if let Some(detail) = &mut state.show_song_detail {
                    if !detail.loading_favorite {
                        if detail.is_favorited {
                            if let Some(fav_id) = detail.favorite_id {
                                match api.remove_favorite(fav_id).await {
                                    Ok(()) => {
                                        detail.is_favorited = false;
                                        detail.favorite_id = None;
                                        state.favorites.items.clear();
                                    }
                                    Err(e) => state.notify(format!("Error: {}", e)),
                                }
                            }
                        } else {
                            let song_id = detail.song.id_str();
                            match api.add_favorite(&song_id).await {
                                Ok(fe) => {
                                    detail.is_favorited = true;
                                    detail.favorite_id = Some(fe.id);
                                    state.favorites.items.clear();
                                }
                                Err(e) => state.notify(format!("Error: {}", e)),
                            }
                        }
                    }
                }
            }
            // Play/stop preview
            KeyCode::Char('p') => {
                if let Some(detail) = &mut state.show_song_detail {
                    if detail.is_previewing {
                        // Stop preview and restart main stream
                        detail.is_previewing = false;
                        state.pending_audio = Some(PendingAudioCommand::Start);
                    } else if let Some(preview_url) = detail.song.preview_src.clone() {
                        detail.is_previewing = true;
                        state.pending_audio = Some(PendingAudioCommand::StartUrl(preview_url));
                    } else {
                        state.notify("No preview available for this song");
                    }
                }
            }
            _ => {}
        }
        return;
    }

    match state.view.clone() {
        View::Login => handle_login_key(key, state, api).await,
        View::NowPlaying => {
            handle_now_playing_key(key, state).await;
            handle_now_playing_actions(key, state, api).await;
        }
        View::History => handle_list_key(key, state, api, "history").await,
        View::Favorites => handle_list_key(key, state, api, "favorites").await,
        View::Charts => handle_list_key(key, state, api, "charts").await,
        View::News => handle_list_key(key, state, api, "news").await,
        View::Profile => handle_profile_key(key, state, api).await,
    }
}

async fn handle_login_key(
    key: crossterm::event::KeyEvent,
    state: &mut AppState,
    api: &mut ApiClient,
) {
    use crossterm::event::KeyCode;

    match key.code {
        KeyCode::Tab => {
            state.login_focus_password = !state.login_focus_password;
        }
        KeyCode::Char('g') | KeyCode::Char('G') => {
            // Continue as guest — audio plays, account features unavailable
            tracing::info!("Continuing as guest");
            state.is_authenticated = false;
            state.view = View::NowPlaying;
            match api.get_status().await {
                Ok(status) => {
                    tracing::info!("Status loaded: {} — {}", status.song.artist, status.song.title);
                    state.artwork_url = status.song.artwork_sm_src.clone()
                        .or_else(|| status.song.artwork_src.clone());
                    state.now_playing = Some(status);
                }
                Err(e) => tracing::warn!("Failed to load status: {}", e),
            }
            state.pending_audio = Some(PendingAudioCommand::Start);
        }
        KeyCode::Enter => {
            let username = state.login_username.clone();
            let password = state.login_password.clone();

            if username.is_empty() || password.is_empty() {
                state.login_error = Some("Username and password required".to_string());
                return;
            }

            tracing::info!("Attempting login for user: {}", username);
            match auth::login(api, &username, &password).await {
                Ok(token) => {
                    tracing::info!("Login successful for user: {}", username);
                    auth::save_token(&token);
                    api.set_token(Some(token));
                    state.login_error = None;
                    state.is_authenticated = true;
                    state.view = View::NowPlaying;
                    match api.get_status().await {
                        Ok(status) => {
                            tracing::info!("Status loaded: {} — {}", status.song.artist, status.song.title);
                            state.artwork_url = status.song.artwork_sm_src.clone()
                                .or_else(|| status.song.artwork_src.clone());
                            state.now_playing = Some(status);
                        }
                        Err(e) => tracing::warn!("Failed to load status after login: {}", e),
                    }
                    state.pending_audio = Some(PendingAudioCommand::Start);
                }
                Err(e) => {
                    tracing::warn!("Login failed for {}: {}", username, e);
                    state.login_error = Some(format!("Login failed: {}", e));
                }
            }
        }
        KeyCode::Char(c) => {
            if state.login_focus_password {
                state.login_password.push(c);
            } else {
                state.login_username.push(c);
            }
        }
        KeyCode::Backspace => {
            if state.login_focus_password {
                state.login_password.pop();
            } else {
                state.login_username.pop();
            }
        }
        _ => {}
    }
}

async fn handle_now_playing_key(
    key: crossterm::event::KeyEvent,
    state: &mut AppState,
) {
    use crossterm::event::KeyCode;

    match key.code {
        KeyCode::Char(' ') => {
            if state.is_playing {
                state.pending_audio = Some(PendingAudioCommand::Pause);
            } else {
                // Resume, or start fresh if never started
                state.pending_audio = Some(PendingAudioCommand::Resume);
            }
        }
        KeyCode::Char('+') | KeyCode::Char('=') => {
            state.volume = (state.volume + 0.05).min(1.0);
            state.notify(format!("Volume: {:.0}%", state.volume * 100.0));
            state.pending_audio = Some(PendingAudioCommand::SetVolume(state.volume));
        }
        KeyCode::Char('-') => {
            state.volume = (state.volume - 0.05).max(0.0);
            state.notify(format!("Volume: {:.0}%", state.volume * 100.0));
            state.pending_audio = Some(PendingAudioCommand::SetVolume(state.volume));
        }
        _ => {}
    }
}

// Note: favorite/reaction actions are auth-protected at the API level already
async fn handle_now_playing_actions(
    key: crossterm::event::KeyEvent,
    state: &mut AppState,
    api: &ApiClient,
) {
    use crossterm::event::KeyCode;

    match key.code {
        KeyCode::Char('f') if state.is_authenticated => {
            if let Some(np) = state.now_playing.as_ref() {
                let song_id = np.song.id_str();
                tracing::info!("Adding favorite: {}", song_id);
                match api.add_favorite(&song_id).await {
                    Ok(_) => state.notify("\u{2665} Added to favorites"),
                    Err(e) => {
                        tracing::warn!("Add favorite failed: {}", e);
                        state.notify(format!("Error: {}", e));
                    }
                }
            }
        }
        KeyCode::Char('r') if state.is_authenticated => {
            tracing::info!("Sending reaction");
            match api.send_reaction(1).await {
                Ok(count) => state.notify(format!("\u{2726} Reaction sent! Total: {}", count)),
                Err(e) => {
                    tracing::warn!("Send reaction failed: {}", e);
                    state.notify(format!("Error: {}", e));
                }
            }
        }
        KeyCode::Char('f') | KeyCode::Char('r') => {
            state.notify("Login required for this action");
        }
        _ => {}
    }
}

async fn handle_list_key(
    key: crossterm::event::KeyEvent,
    state: &mut AppState,
    api: &ApiClient,
    view_name: &str,
) {
    use crossterm::event::KeyCode;

    let (len, selected) = match view_name {
        "history" => (state.history.items.len(), state.history.selected),
        "favorites" => (state.favorites.items.len(), state.favorites.selected),
        "charts" => (state.charts.items.len(), state.charts.selected),
        "news" => (state.news.items.len(), state.news.selected),
        _ => return,
    };

    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            let new_sel = (selected + 1).min(len.saturating_sub(1));
            match view_name {
                "history" => state.history.selected = new_sel,
                "favorites" => state.favorites.selected = new_sel,
                "charts" => state.charts.selected = new_sel,
                "news" => state.news.selected = new_sel,
                _ => {}
            }
            // Auto-load next page when reaching the last item
            if new_sel == len.saturating_sub(1) && len > 0 {
                match view_name {
                    "favorites" if state.favorites.page < state.favorites.last_page => {
                        let next = state.favorites.page + 1;
                        tracing::info!("Loading favorites page {}", next);
                        match api.get_favorites(next).await {
                            Ok(f) => {
                                state.favorites.items.extend(f.data);
                                state.favorites.last_page = f.meta.last_page;
                                state.favorites.page = next;
                            }
                            Err(e) => tracing::warn!("Favorites page {} load failed: {}", next, e),
                        }
                    }
                    "history" if state.history.page < state.history.last_page => {
                        let next = state.history.page + 1;
                        tracing::info!("Loading history page {}", next);
                        match api.get_history(next).await {
                            Ok(h) => {
                                state.history.items.extend(h.data);
                                state.history.last_page = h.meta.last_page;
                                state.history.page = next;
                            }
                            Err(e) => tracing::warn!("History page {} load failed: {}", next, e),
                        }
                    }
                    _ => {}
                }
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            let new_sel = selected.saturating_sub(1);
            match view_name {
                "history" => state.history.selected = new_sel,
                "favorites" => state.favorites.selected = new_sel,
                "charts" => state.charts.selected = new_sel,
                "news" => state.news.selected = new_sel,
                _ => {}
            }
        }
        KeyCode::Char('g') => {
            match view_name {
                "history" => state.history.selected = 0,
                "favorites" => state.favorites.selected = 0,
                "charts" => state.charts.selected = 0,
                "news" => state.news.selected = 0,
                _ => {}
            }
        }
        KeyCode::Char('G') => {
            let bottom = len.saturating_sub(1);
            match view_name {
                "history" => state.history.selected = bottom,
                "favorites" => state.favorites.selected = bottom,
                "charts" => state.charts.selected = bottom,
                "news" => state.news.selected = bottom,
                _ => {}
            }
        }
        KeyCode::Enter if view_name == "history" && selected < len => {
            let entry = &state.history.items[selected];
            state.show_song_detail = Some(SongDetailState::new(entry.song.clone(), entry.played_at));
        }
        KeyCode::Enter if view_name == "charts" && selected < len => {
            let song = state.charts.items[selected].song.clone();
            state.show_song_detail = Some(SongDetailState::new(song, None));
        }
        KeyCode::Char('f') if view_name == "history" && selected < len => {
            if state.is_authenticated {
                let song_id = state.history.items[selected].song.id_str();
                tracing::info!("Adding history item to favorites: {}", song_id);
                match api.add_favorite(&song_id).await {
                    Ok(_) => state.notify("\u{2665} Added to favorites"),
                    Err(e) => state.notify(format!("Error: {}", e)),
                }
            } else {
                state.notify("Login required to favorite songs");
            }
        }
        KeyCode::Char('d') | KeyCode::Delete if view_name == "favorites" && selected < len => {
            let fav_id = state.favorites.items[selected].id;
            tracing::info!("Removing favorite id {}", fav_id);
            match api.remove_favorite(fav_id).await {
                Ok(_) => {
                    state.favorites.items.remove(selected);
                    if selected > 0 {
                        state.favorites.selected = selected - 1;
                    }
                    state.notify("Removed from favorites");
                }
                Err(e) => state.notify(format!("Error: {}", e)),
            }
        }
        KeyCode::Char('h') | KeyCode::Left if view_name == "charts" => {
            state.chart_range = match state.chart_range {
                RatingRange::Overtime => RatingRange::Monthly,
                RatingRange::Weekly => RatingRange::Overtime,
                RatingRange::Monthly => RatingRange::Weekly,
            };
            state.charts.items.clear();
            state.charts.selected = 0;
            tracing::info!("Switching charts to {:?}", state.chart_range.as_str());
            if let Ok(c) = api.get_ratings(state.chart_range, 1).await {
                state.charts.items = c.data;
                state.charts.last_page = c.meta.last_page;
                state.charts.page = 1;
            }
        }
        KeyCode::Char('l') | KeyCode::Right if view_name == "charts" => {
            state.chart_range = match state.chart_range {
                RatingRange::Overtime => RatingRange::Weekly,
                RatingRange::Weekly => RatingRange::Monthly,
                RatingRange::Monthly => RatingRange::Overtime,
            };
            state.charts.items.clear();
            state.charts.selected = 0;
            tracing::info!("Switching charts to {:?}", state.chart_range.as_str());
            if let Ok(c) = api.get_ratings(state.chart_range, 1).await {
                state.charts.items = c.data;
                state.charts.last_page = c.meta.last_page;
                state.charts.page = 1;
            }
        }
        _ => {}
    }
}

async fn handle_profile_key(
    key: crossterm::event::KeyEvent,
    state: &mut AppState,
    _api: &ApiClient,
) {
    use crossterm::event::KeyCode;

    match key.code {
        KeyCode::Char('L') => {
            tracing::info!("User logging out");
            auth::delete_token();
            state.user = None;
            state.user_stats = None;
            state.now_playing = None;
            state.history.items.clear();
            state.favorites.items.clear();
            state.is_authenticated = false;
            state.is_playing = false;
            state.view = View::Login;
            state.notify("Logged out");
        }
        _ => {}
    }
}

fn render(frame: &mut ratatui::Frame, state: &AppState, artwork: &mut Option<StatefulProtocol>, popup_artwork: &mut Option<StatefulProtocol>) {
    use crate::tui::views;
    use ratatui::style::Style;
    use ratatui::widgets::{Block, Clear};

    let area = frame.area();

    // Clear entire frame first — ensures stale characters from popups/overlays
    // in the previous frame are overwritten before any widget renders.
    frame.render_widget(Clear, area);

    // Fill background
    frame.render_widget(
        Block::default().style(Style::default().bg(crate::theme::Theme::BACKGROUND)),
        area,
    );

    if state.view == View::Login {
        views::login::render(frame, area, state);
        return;
    }

    let layout = AppLayout::new(area);

    // Render header
    render_header(frame, layout.header, state);

    // Render sidebar
    render_sidebar(frame, layout.sidebar, state);

    // Clear content area before rendering view to prevent bleed from previous view
    frame.render_widget(Clear, layout.content);

    // Render content
    match state.view {
        View::NowPlaying => views::now_playing::render(frame, layout.content, state, artwork),
        View::History => views::history::render(frame, layout.content, state),
        View::Favorites => views::favorites::render(frame, layout.content, state),
        View::Charts => views::charts::render(frame, layout.content, state),
        View::News => views::news::render(frame, layout.content, state),
        View::Profile => views::profile::render(frame, layout.content, state),
        View::Login => unreachable!(),
    }

    // Render status bar
    render_status_bar(frame, layout.status_bar, state);

    // Overlays
    if state.show_help {
        views::help::render(frame, area, state);
    }

    if let Some(detail) = &state.show_song_detail {
        crate::tui::widgets::render_song_detail_popup(frame, detail, popup_artwork);
    }
}

fn render_header(
    frame: &mut ratatui::Frame,
    area: ratatui::layout::Rect,
    state: &AppState,
) {
    use ratatui::layout::Alignment;
    use ratatui::text::{Line, Span, Text};
    use ratatui::widgets::Paragraph;
    use crate::theme::Theme;

    let logo = "\u{266b} NIGHTWAVE PLAZA";
    let song_info = state
        .now_playing
        .as_ref()
        .map(|np| np.song.display_name())
        .unwrap_or_else(|| "Connecting...".to_string());

    let text = Text::from(vec![
        Line::from(Span::styled(logo, Theme::pink_bold())),
        Line::from(Span::styled(&song_info, Theme::cyan_bold())),
    ]);

    let block = crate::tui::widgets::vaporwave_block("Plaza TUI");
    let para = Paragraph::new(text).block(block).alignment(Alignment::Center);
    frame.render_widget(para, area);
}

fn render_sidebar(
    frame: &mut ratatui::Frame,
    area: ratatui::layout::Rect,
    state: &AppState,
) {
    use ratatui::text::{Line, Span};
    use ratatui::widgets::{Block, Borders, List, ListItem};
    use crate::theme::Theme;

    let items: Vec<ListItem> = View::nav_items()
        .iter()
        .map(|(label, view)| {
            let style = if *view == state.view {
                Theme::selected()
            } else {
                Theme::text()
            };
            ListItem::new(Line::from(Span::styled(*label, style)))
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Theme::dim())
            .style(ratatui::style::Style::default().bg(Theme::BACKGROUND)),
    );
    frame.render_widget(list, area);
}

fn render_status_bar(
    frame: &mut ratatui::Frame,
    area: ratatui::layout::Rect,
    state: &AppState,
) {
    use ratatui::text::{Line, Span};
    use ratatui::widgets::Paragraph;
    use crate::theme::Theme;

    let connection_span = match state.connection {
        ConnectionStatus::Connected => Span::styled(
            "\u{25cf} LIVE",
            ratatui::style::Style::default().fg(Theme::GREEN),
        ),
        ConnectionStatus::Disconnected => Span::styled(
            "\u{2717} DISCONNECTED",
            ratatui::style::Style::default().fg(Theme::RED),
        ),
        ConnectionStatus::Connecting => Span::styled(
            "\u{25cc} CONNECTING",
            ratatui::style::Style::default().fg(Theme::YELLOW),
        ),
    };

    let listeners = state
        .now_playing
        .as_ref()
        .map(|np| format!(" | \u{25c9} {} listening", np.listeners))
        .unwrap_or_default();

    let play_state = if state.is_playing { " | \u{25b6} Playing" } else { "" };
    let volume = format!(" | Vol: {:.0}%", state.volume * 100.0);
    let auth_note = if !state.is_authenticated { " | Guest" } else { "" };

    let notification = state
        .notification
        .as_ref()
        .map(|(msg, _)| format!(" | {}", msg))
        .unwrap_or_default();

    let hints = "  [?] Help [q] Quit";

    let line = Line::from(vec![
        connection_span,
        Span::styled(&listeners, Theme::dim()),
        Span::styled(play_state, ratatui::style::Style::default().fg(Theme::GREEN)),
        Span::styled(&volume, Theme::dim()),
        Span::styled(auth_note, Theme::dim()),
        Span::styled(
            &notification,
            ratatui::style::Style::default().fg(Theme::YELLOW),
        ),
        Span::styled(hints, Theme::dim()),
    ]);

    let para =
        Paragraph::new(line).style(ratatui::style::Style::default().bg(Theme::BACKGROUND));
    frame.render_widget(para, area);
}

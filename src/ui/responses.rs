//! What the backend answers, applied to the app.

use std::collections::HashSet;
use std::time::Duration;

use souvlaki::MediaControlEvent;

use crate::audio::{AudioCmd, AudioEvent};
use crate::backend::{Req, Resp};
use crate::model::{ShelfKind, Track};
use crate::ytm::HomePage;

use super::{Action, App, Dialog, HomeState, LibTab, MixFrom, Page, PlayingAs, dev_var, shuffle};

impl App {
    pub(super) fn pump(&mut self, ctx: &egui::Context, now: f64) {
        // Sorted playlists load again when it's time, for their new order. One whose load
        // ended without its first page (it failed) stops waiting, so Sort comes back.
        let mut due = Vec::new();
        let lists = &self.lists;
        self.sorting.retain(|id, w| {
            if w.take_due(now) {
                due.push(id.clone());
                return true;
            }
            w.awaiting() && lists.loading(id)
        });
        for id in due {
            self.load_list(&id, false, true);
        }
        let cover = self.media_cover.lock().unwrap().take();
        // The listed track's art.
        if let Some((id, file)) = cover
            && self.current().is_some_and(|t| t.id == id)
            && let Some(m) = &mut self.media
        {
            m.set_cover(&file);
        }
        while let Ok(r) = self.ch.resp_rx.try_recv() {
            self.on_resp(r, now);
        }
        while let Ok(ev) = self.ch.audio_rx.try_recv() {
            match ev {
                AudioEvent::Finished { generation } if generation == self.generation && self.jam.guest() => {}
                AudioEvent::Finished { generation } if generation == self.generation => {
                    if self.repeats_one() {
                        self.play_current();
                    } else {
                        self.next();
                    }
                }
                AudioEvent::Finished { .. } => {}
                // Crossfade into the next song; repeat-one and the end of the queue wait for the end.
                AudioEvent::Ending { generation } if generation == self.generation && !self.jam.guest() && !self.repeats_one() => {
                    // Automix: the next song eases in under this one, which keeps the
                    // player until the blend has given the next song the louder half
                    // (see `MixFrom`).
                    if let Some(from) = self.playing_track().cloned() {
                        let total = self
                            .audio
                            .status
                            .duration()
                            .map(|d| d.as_secs_f32())
                            .or_else(|| from.duration.map(|d| d as f32))
                            .unwrap_or(0.0);
                        let mix = self.audio.status.mix_secs(Some(std::time::Duration::from_secs_f32(total)));
                        if total > 0.0 && mix > 0.5 {
                            self.pending_mix = Some(MixFrom { track: from, total, mix, started: false, released: false });
                        }
                    }
                    let before = self.playing_id().map(str::to_owned);
                    self.handoff = true;
                    self.next();
                    self.handoff = false;
                    if self.playing_id().map(str::to_owned) != before {
                        self.mix_from = self.pending_mix.take();
                    } else {
                        self.pending_mix = None;
                    }
                }
                AudioEvent::Ending { .. } => {}
                AudioEvent::Interrupted { generation, at, reason } if generation == self.generation => self.recover(at, &reason),
                AudioEvent::Interrupted { .. } => {}
                AudioEvent::Error(e) => self.toast(e),
            }
        }
        // Once the next song is the louder of the two, it takes the player over; the
        // caption stays while the handoff's tail runs out.
        let released = self.mix_from.as_ref().is_some_and(|h| h.released);
        let switched = self.watch_mix();
        let over = self.mix_over();
        if (switched || over) && !released {
            self.release_mix();
        }
        if over {
            self.mix_from = None;
        }
        while let Ok(ev) = self.ch.media_rx.try_recv() {
            match ev {
                MediaControlEvent::Play | MediaControlEvent::Pause if self.jam.guest() => self.toggle(),
                MediaControlEvent::Play if self.resume_at.is_some() => self.toggle(),
                MediaControlEvent::Play => self.audio.send(AudioCmd::Resume),
                MediaControlEvent::Pause | MediaControlEvent::Stop => self.audio.send(AudioCmd::Pause),
                MediaControlEvent::Toggle => self.toggle(),
                MediaControlEvent::Next => self.next(),
                MediaControlEvent::Previous => self.prev(),
                // The window from the OS's media controls (KDE's, say; GNOME's use the .desktop file).
                MediaControlEvent::Raise => self.show_window(ctx),
                MediaControlEvent::Quit => self.quit(ctx),
                MediaControlEvent::SetVolume(v) => {
                    if let Some(v) = crate::media::volume_from_os(v) {
                        self.audio.set_volume(v);
                    }
                }
                // Seeks go the way the seek bar's do: to the host in a jam, to the start
                // point of a song not loaded yet, and none without a length to keep them in.
                ev => {
                    let len = self.audio.status.duration().or(self.current().and_then(|t| t.duration).map(|d| Duration::from_secs(d as u64)));
                    if let Some(len) = len
                        && let Some(at) = crate::media::seek_target(&ev, self.audio.status.position(), len)
                    {
                        self.apply(Action::Seek(at.as_secs_f32()));
                    }
                }
            }
        }
        if self.buffering && self.audio.status.started(self.generation) {
            self.buffering = false;
            self.skip_streak = 0;
            self.prefetch_next();
            // Dev hook: YTM_SEEK=<secs> jumps into the first track once it starts.
            if self.generation == 1 {
                if let Some(s) = std::env::var("YTM_SEEK").ok().and_then(|v| v.parse::<f32>().ok()) {
                    self.audio.send(AudioCmd::Seek(Duration::from_secs_f32(s)));
                }
                // YTM_SEEK_LATER=<secs>@<delay> seeks again that many seconds later.
                if let Some((s, d)) = dev_var("YTM_SEEK_LATER").and_then(|v| v.split_once('@')).and_then(|(s, d)| Some((s.parse::<f32>().ok()?, d.parse::<f32>().ok()?))) {
                    let audio = self.audio.clone();
                    std::thread::spawn(move || {
                        std::thread::sleep(Duration::from_secs_f32(d));
                        audio.send(AudioCmd::Seek(Duration::from_secs_f32(s)));
                    });
                }
            }
        }
        if self.suggest_due.is_some_and(|d| now >= d) {
            self.suggest_due = None;
            let q = self.search_text.trim().to_owned();
            // An empty box signed in shows the account's recent searches.
            if self.forgets_pending == 0 && (!q.is_empty() || self.logged_in) {
                self.backend.request(Req::Suggest { query: q, round: self.suggest_round });
            }
        }
    }

    /// A search's results are on screen: the first result clicked becomes what
    /// "Recently Searched" remembers, not the words that were typed.
    pub(super) fn expect_search_click(&mut self, query: &str) {
        if matches!(&self.page, Page::Search(q) if q == query) {
            self.search_pending = true;
        }
    }

    /// The entry a click makes: the item itself, with the artwork it was shown with.
    fn search_click_entry(&self, a: &Action) -> Option<super::RecentSearch> {
        let r = self.search.as_ref()?;
        let best = |thumbs: &[crate::model::Thumb]| thumbs.iter().max_by_key(|t| t.width).map(|t| t.url.clone());
        let song = |t: &Track| {
            Some(super::RecentSearch {
                query: r.query.clone(),
                target: Some(crate::model::Target::Song(t.clone())),
                title: t.title.clone(),
                subtitle: format!("Song • {}", t.artist_line()),
                thumb: best(&t.thumbs),
                round: false,
            })
        };
        let card = |target: &crate::model::Target| {
            let c = r.artists.iter().chain(&r.albums).chain(&r.playlists).find(|c| c.target == *target)?;
            Some(super::RecentSearch {
                query: r.query.clone(),
                target: Some(target.clone()),
                title: c.title.clone(),
                subtitle: c.subtitle.clone(),
                thumb: best(&c.thumbs),
                round: c.round,
            })
        };
        match a {
            Action::Open(crate::model::Target::Song(t)) | Action::Radio(t) => song(t),
            Action::Open(target) | Action::PlayCollection(target, _) => card(target),
            Action::Play(tracks, i) => tracks.get(*i).and_then(song),
            _ => None,
        }
    }

    /// Record the thing the search's first click opened, once.
    pub(super) fn remember_search_click(&mut self, a: &Action) {
        if !self.search_pending || !matches!(self.page, Page::Search(_)) {
            return;
        }
        let Some(entry) = self.search_click_entry(a) else { return };
        self.search_pending = false;
        // One entry per thing; a bare query left by an older version goes with it.
        self.recent_searches.retain(|s| {
            s.target.as_ref() != entry.target.as_ref() && !(s.target.is_none() && s.query.eq_ignore_ascii_case(&entry.query))
        });
        self.recent_searches.insert(0, entry);
        self.recent_searches.truncate(8);
    }

    fn on_resp(&mut self, r: Resp, now: f64) {
        match r {
            Resp::Home(page) => self.on_home(page),
            Resp::HomeMore(page) => {
                if let Some(h) = &mut self.home {
                    h.shelves.extend(page.shelves);
                    h.continuation = page.continuation;
                    h.loading_more = None;
                }
            }
            Resp::Explore(e) => {
                self.explore = Some(e);
                self.loading = false;
            }
            Resp::Mood(m) => {
                if self.page == Page::Mood(m.params.clone()) {
                    self.loading = false;
                }
                self.mood = Some(m);
            }
            Resp::VideoStream { video_id, max_height, stream } => {
                // Not one asked for before the quality changed.
                if let Some((id, player)) = &mut self.video
                    && *id == video_id
                    && max_height == self.video_quality
                {
                    let none = stream.is_none();
                    *player = stream.map(|s| crate::video::Player::start(s, self.backend.http(), self.audio.status.clone()));
                    if none {
                        self.toast("This video can't be shown");
                    }
                }
            }
            Resp::SearchIn(r) => {
                self.search = Some(r);
                self.loading = false;
            }
            Resp::Search(r) => {
                if self.smoke.as_ref() == Some(&r.query) {
                    self.smoke = None;
                    if !r.tracks.is_empty() {
                        log::info!("smoke: playing {}", r.tracks[0].title);
                        self.play_list(r.tracks.clone(), 0);
                    }
                }
                self.expect_search_click(&r.query);
                self.search = Some(r);
                self.loading = false;
            }
            Resp::SearchForgotten => {
                // Replies dropped meanwhile left the list behind what's typed: ask again.
                self.forgets_pending = self.forgets_pending.saturating_sub(1);
                if self.forgets_pending == 0 {
                    self.suggest_due = Some(now);
                }
            }
            Resp::Suggest { query, round, terms } => {
                if round == self.suggest_round && self.forgets_pending == 0 {
                    // Keep late answers for backspacing/revisiting, but never display
                    // an answer for a query other than the one currently in the box.
                    self.suggest_cache.retain(|(q, _)| q != &query);
                    self.suggest_cache.push_back((query.clone(), terms.clone()));
                    if self.suggest_cache.len() > 32 {
                        self.suggest_cache.pop_front();
                    }
                    if self.search_text.trim() == query {
                        self.suggestions = (query, terms);
                        self.suggest_sel = None;
                    }
                }
            }
            Resp::Collection { load, c, more } => {
                let id = c.id.clone();
                self.collection_in(load, c, more);
                // A sort waiting for its order, from this load (an older one isn't kept).
                let sorted = self.lists.get(&id).filter(|o| o.load == load).zip(self.sorting.get_mut(&id)).is_some_and(|(o, w)| w.arrived(&o.c.sorts, now));
                if sorted {
                    self.sorting.remove(&id);
                }
            }
            Resp::CollectionMore { id, load, tracks, set_ids, more } => self.collection_more(id, load, tracks, set_ids, more),
            Resp::Artist(mut a) => {
                // A slow answer for an artist left meanwhile doesn't replace the one on screen.
                if matches!(&self.page, Page::Artist(id) if *id != a.id)
                    && !self.shelf_pending.as_ref().is_some_and(|(id, _)| *id == a.id)
                {
                    return;
                }
                if self.page == Page::Artist(a.id.clone()) {
                    self.loading = false;
                }
                // Asked for before a subscribe here was answered, it can be from before that.
                if let Some(channel) = &a.channel_id {
                    a.subscribed = self.subscriptions.shown(channel, a.subscribed, a.asked);
                }
                // A shelf page that opened before its artist page was here gets its
                // request out now, with the More link the page carries.
                let pending = self.shelf_pending.take_if(|(id, _)| *id == a.id);
                self.artist = Some(a);
                if let Some((id, title)) = pending {
                    if let Some(req) = self.open_shelf(&id, &title) {
                        self.backend.request(req);
                    }
                }
            }
            Resp::ArtistShelf { artist, title, cards } => {
                // Only the shelf that asked shows the answer: its key rode with the
                // request, so a slow answer for a shelf left meanwhile is dropped.
                let filled = self.shelf.as_mut().filter(|s| s.key == (artist.clone(), title.clone()) && s.loading).map(|s| {
                    s.cards = cards;
                    s.loading = false;
                });
                if filled.is_some() && self.page == (Page::ArtistShelf { artist: artist.clone(), title: title.clone() }) {
                    self.loading = false;
                }
            }
            Resp::Library(mut l) => {
                // A half that didn't load keeps what was shown (or last launch's sidebar),
                // rather than emptying it.
                if l.playlists_failed {
                    l.playlists = self.library.as_ref().map(|o| o.playlists.clone()).or_else(|| self.sidebar_cache.clone()).unwrap_or_default();
                }
                if l.albums_failed {
                    l.albums = self.library.as_ref().map(|o| o.albums.clone()).unwrap_or_default();
                }
                if l.artists_failed {
                    l.artists = self.library.as_ref().map(|o| o.artists.clone()).unwrap_or_default();
                }
                self.library = Some(l);
                self.loading = false;
            }
            Resp::History(h) => {
                self.history = Some(h);
                self.loading = false;
            }
            Resp::Uploads(t) => {
                self.uploads = Some(t);
                if self.page == Page::Library(LibTab::Uploads) {
                    self.loading = false;
                }
            }
            Resp::Downloaded { title, path } => {
                log::info!("downloaded to {}", path.display());
                self.toast(format!("Downloaded \u{201c}{title}\u{201d}"));
                self.all_downloaded.clear();
                if self.page == Page::Downloads {
                    self.reload_downloads();
                }
            }
            Resp::Downloads(runs) => self.on_download_runs(runs),
            Resp::ConfirmDownload { target, title, tracks, count } => self.dialog = Some(Dialog::ConfirmDownload { target, title, tracks, count }),
            Resp::Uploaded { done, failed } => {
                let songs = |n: usize| if n == 1 { "1 song".to_owned() } else { format!("{n} songs") };
                self.toast(match (done, failed.len()) {
                    (_, 0) => format!("Uploaded {}. It can take a minute for YouTube Music to process them.", songs(done)),
                    (0, _) => format!("Upload failed: {}", failed.join(", ")),
                    _ => format!("Uploaded {}; failed: {}", songs(done), failed.join(", ")),
                });
            }
            Resp::Lyrics { video_id, lyrics } => {
                if self.shown().is_some_and(|t| t.id == video_id) {
                    let available = self.lyric_prefs.available(&lyrics);
                    let shown = self.lyric_prefs.choose(&video_id, &lyrics);
                    let missing: Vec<&str> = self.lyric_prefs.order.iter().filter(|(p, on)| *on && !available.contains(p)).map(|(p, _)| p.key()).collect();
                    log::info!(
                        "lyrics: showing {} ({}/{}); available: {}; no lyrics from: {}",
                        shown.map_or("nothing", |p| p.key()),
                        shown.and_then(|p| available.iter().position(|x| *x == p)).map_or(0, |i| i + 1),
                        available.len(),
                        available.iter().map(|p| p.key()).collect::<Vec<_>>().join(", "),
                        missing.join(", ")
                    );
                    self.lyrics = Some((video_id, lyrics));
                }
            }
            Resp::CommentsPage { token, page } => super::comments::on_page(&token, page),
            Resp::Segments { video_id, segments } => {
                if self.playing_id() == Some(video_id.as_str()) {
                    log::info!("sponsorblock: {} segment(s) for {video_id}", segments.len());
                    self.segments = Some((video_id, segments));
                }
            }
            Resp::LyricsExtras { video_id, source, target, extras } => {
                if self.shown().is_some_and(|t| t.id == video_id) {
                    self.lyrics_extras = Some((video_id, source, target, extras));
                }
            }
            Resp::Art { key, art } => {
                if self.mix_hold().is_none() && self.current_id() == Some(key.as_str()) {
                    self.accent = art.accent;
                    if let Some(bg) = &self.bg {
                        bg.set_art(art);
                    }
                    self.art_key = Some(key);
                    self.art_changed = now;
                }
            }
            Resp::CanvasUrl { id, url } => {
                // Apple's answer is in (`canvas` remembers a miss on disk): a found
                // clip goes up, a miss hands the song to Spotify's canvas.
                self.canvas_apple_waiting.remove(&id);
                self.canvas_apple_done.insert(id.clone());
                if let Some(url) = url {
                    self.canvas_covers.insert(id, url);
                }
            }
            Resp::ArtistSignature { key, signature } => {
                self.signature_waiting.remove(&key);
                match signature {
                    Some(s) => {
                        self.signature_missed.remove(&key);
                        self.signature_images.insert(key, (s.width, s.height, s.rgba));
                    }
                    None => {
                        self.signature_missed.insert(key, std::time::Instant::now());
                    }
                }
            }
            Resp::SpotifyCanvas { key, url } => {
                self.canvas_waiting.remove(&key);
                match url {
                    Some(url) => {
                        self.canvas_missed.remove(&key);
                        self.canvas_covers.insert(key, url);
                    }
                    None => {
                        self.canvas_missed.insert(key, std::time::Instant::now());
                    }
                }
            }
            Resp::Radio { seed, tracks } => {
                self.radio_pending = false;
                let was_at_end = self.queue.add_radio(&seed, tracks);
                if was_at_end && self.queue.index.is_some() && !self.buffering && !self.playing() {
                    // The track ended while radio was loading: continue now.
                    self.next();
                } else if !self.buffering {
                    self.prefetch_next();
                }
            }
            // Unless something else was asked to play meanwhile; shuffled as last asked.
            Resp::PlayQueue { target, title, mut tracks } => {
                if let Some((p, shuffled)) = &self.play_pending
                    && *p == target
                {
                    if *shuffled {
                        shuffle(&mut tracks);
                    }
                    self.played_title = Some((target.clone(), title));
                    self.queue_source = Some(target);
                    self.play_list(tracks, 0);
                }
            }
            Resp::Liked { video_id, like } => {
                self.lists.edits.confirm_likes(std::slice::from_ref(&video_id), &[]);
                if like {
                    self.liked.insert(video_id);
                }
            }
            Resp::QuickPicksMore { seed, tracks } => {
                if let Some(h) = &mut self.home {
                    h.picks_loading = false;
                    if let Some(shelf) = h.shelves.iter_mut().find(|s| s.title == "Quick picks")
                        && let ShelfKind::Songs { tracks: picks, .. } = &mut shelf.kind
                        && picks.iter().any(|t| t.id == seed)
                    {
                        h.picks_expanded = true;
                        let mut seen: HashSet<String> = picks.iter().map(|t| t.id.clone()).collect();
                        picks.extend(tracks.into_iter().filter(|t| seen.insert(t.id.clone())).take(24));
                    }
                }
            }
            Resp::LikeProgress { taken, done, total, like } => {
                if !like {
                    let tracks: Vec<Track> = taken.iter().filter_map(|id| self.unliking.remove(id)).collect();
                    self.set_liked(&tracks, false);
                }
                if done == 0 {
                    // A Spotify transfer is adding to Liked Music (maybe paused): this waits for it.
                    self.toast("Waiting for the transfer to finish adding to Liked Music…");
                } else if total > 1 && done < total {
                    let note = if self.backend.api().limited() { " (YouTube Music is limiting requests; waiting)" } else { "" };
                    self.toast(format!("{} {done} of {total} songs…{note}", if like { "Liking" } else { "Removing likes:" }));
                }
            }
            Resp::LikedMany { video_ids, failed, like } => self.on_liked_many(video_ids, failed, like),
            Resp::PlaylistCreated { id, songs } => {
                self.request_library();
                self.toast(match songs {
                    0 => "Playlist created".to_owned(),
                    1 => "Song added to new playlist".to_owned(),
                    n => format!("{n} songs added to new playlist"),
                });
                self.go(Page::Playlist(id));
            }
            Resp::PlaylistChanged { id, message } => self.on_playlist_changed(id, message),
            Resp::Subscribed { channel_id, on, ok } => {
                // As it now stands, over a reload fetched before the change.
                let now = self.subscriptions.finish(&channel_id, on, ok);
                if let Some(a) = self.artist.as_mut().filter(|a| a.channel_id.as_deref() == Some(&channel_id)) {
                    a.subscribed = now;
                }
                // Library › Artists lists it now (or no longer).
                if ok {
                    self.request_library();
                }
                self.toast(match (ok, on) {
                    (true, true) => "Subscribed",
                    (true, false) => "Unsubscribed",
                    (false, true) => "Couldn't subscribe",
                    (false, false) => "Couldn't unsubscribe",
                });
            }
            Resp::Collaboration(c) => {
                if let Some(Dialog::Collaborate { id, info, busy, .. }) = &mut self.dialog
                    && *id == c.playlist_id
                {
                    *info = Some(c);
                    *busy = false;
                }
            }
            Resp::PlayingAs { generation, listed, song } => {
                // For the track it was looked up for: joining a jam swaps the queue without a new play.
                if generation == self.generation
                    && self.current_id() == Some(listed.as_str())
                    && let Some(track) = self.current().cloned()
                {
                    let shown = match song {
                        // The song keeps the listed track's album and art when it has none.
                        Some(song) => {
                            let song = Track { album: song.album.or(track.album.clone()), thumbs: if song.thumbs.is_empty() { track.thumbs.clone() } else { song.thumbs }, ..song };
                            self.playing_as = Some(PlayingAs { generation, listed: track.id.clone(), song: song.clone() });
                            song
                        }
                        None => {
                            self.playing_as = None;
                            track
                        }
                    };
                    if self.mix_hold().is_none() {
                        self.show_in_media(&shown);
                        self.refresh_lyrics(shown);
                    }
                    // A jam began while it was being looked up: everyone plays the host's upload.
                    if self.jam.active() {
                        self.play_listed_upload();
                    }
                }
            }
            Resp::MusicVideo { song, video } => {
                self.music_videos.insert(song, video);
            }
            Resp::PlayError { generation, msg } => self.on_play_error(generation, msg),
            Resp::AudioInfo { generation, info } => {
                // The stream that answered may already have been skipped past.
                if generation == self.generation {
                    self.audio_info = Some((generation, info));
                }
            }
            Resp::ServerQueue(q) => self.adopt_server_queue(q),
            Resp::Account { name, photo } => {
                self.account_name = name.or(self.account_name.take());
                self.account_avatar = photo.or(self.account_avatar.take());
            }
            Resp::Channels(c) => self.channels = Some(c),
            Resp::SwitchedChannel { page_id, epoch, error } => {
                let name = self.switching_to.take().unwrap_or_else(|| "another channel".to_owned());
                match error {
                    Some(e) => self.toast(e),
                    None => {
                        // Everything reloads as after signing in, but it's the same sign-in
                        // with the same channels: only which one is used changed.
                        let channels = self.channels.take().map(|c| crate::backend::Channels { current: page_id, epoch, ..c });
                        let asked = self.channels_asked;
                        self.on_resp(Resp::LoggedIn(true), now);
                        (self.channels, self.channels_asked) = (channels, asked);
                        self.toast(format!("Switched to {name}"));
                    }
                }
            }
            Resp::LoggedIn(v) => self.on_logged_in(v, now),
            Resp::Error(e) => self.on_error(e),
        }
    }

    /// The home feed, replacing the one shown.
    fn on_home(&mut self, page: HomePage) {
        // Until the account menu answers (Req::Account).
        if self.account_avatar.is_none() {
            self.account_avatar = page.shelves.iter().find(|s| s.title == "Listen again").and_then(|s| s.strap_thumb.clone());
        }
        let mut chips = page.chips;
        // The feed marks the active mood itself; keep the bar when it doesn't send chips.
        if chips.is_empty() {
            chips = self.home.as_ref().map(|h| h.chips.clone()).unwrap_or_default();
        }
        self.home = Some(HomeState {
            chips,
            shelves: page.shelves,
            continuation: page.continuation,
            loading_more: None,
            picks_loading: false,
            picks_expanded: false,
        });
        self.home_loading = false;
        if self.page == Page::Home {
            self.loading = false;
        }
    }

    /// The Downloads page read whole: every file looked at (disk work, so on a visit,
    /// not as each song lands).
    pub(super) fn read_downloads(&mut self) {
        // Counted first: a song saved meanwhile is taken on again next time, not missed.
        self.downloads_seen = crate::downloads::saves();
        self.downloads = Some(crate::downloads::list());
    }

    /// The Downloads page takes on the songs saved since it was read, at the top, from
    /// the index (no look at the other files). New files come first, so the picked rows
    /// move: the same songs stay picked.
    fn reload_downloads(&mut self) {
        let Some(old) = self.downloads.take() else { return self.read_downloads() };
        let (seen, mut new) = crate::downloads::saved_since(self.downloads_seen);
        self.downloads_seen = seen;
        if new.is_empty() {
            self.downloads = Some(old);
            return;
        }
        let fresh: HashSet<String> = new.iter().map(|t| t.id.clone()).collect();
        new.extend(old.iter().filter(|t| !fresh.contains(&t.id)).cloned());
        if self.selection.page == Some(Page::Downloads) {
            let id = |i: usize| old.get(i).map(|t| t.id.as_str());
            let at = |id: &str| new.iter().position(|t| t.id == id);
            let picked: HashSet<&str> = self.selection.rows.iter().filter_map(|&i| id(i)).collect();
            self.selection.rows = new.iter().enumerate().filter(|(_, t)| picked.contains(t.id.as_str())).map(|(i, _)| i).collect();
            self.selection.anchor = self.selection.anchor.and_then(id).and_then(at);
        }
        self.downloads = Some(new);
    }

    /// Downloads of albums, playlists and selections moved on. One toast as each starts
    /// and one as it ends, not one per song: their pages and the Downloads page show
    /// how far they are.
    fn on_download_runs(&mut self, runs: Vec<crate::backend::DownloadRun>) {
        let mut notes = Vec::new();
        for r in &runs {
            let name = if r.title.is_empty() { String::new() } else { format!(" \u{201c}{}\u{201d}", r.title) };
            if r.finished() {
                notes.push(download_summary(r, &name));
            } else if !self.download_runs.iter().any(|o| o.key == r.key) {
                notes.push(format!("Downloading {}{}", songs(r.total - r.done), if name.is_empty() { String::new() } else { format!(" from{name}") }));
            }
        }
        // Lists that finished are looked at again for their "Downloaded".
        if runs.iter().any(|r| r.finished()) {
            self.all_downloaded.clear();
        }
        let saved = |rs: &[crate::backend::DownloadRun], key: &str| rs.iter().find(|r| r.key == key).map_or(0, |r| r.saved);
        let more = runs.iter().any(|r| r.saved > saved(&self.download_runs, &r.key));
        self.download_runs = runs.into_iter().filter(|r| !r.finished()).collect();
        if !notes.is_empty() {
            self.toast(notes.join(" \u{b7} "));
        }
        if more && self.page == Page::Downloads {
            self.reload_downloads();
        }
    }

    /// Liking or unliking many songs at once has finished: `failed` are the ones YouTube
    /// Music refused.
    fn on_liked_many(&mut self, video_ids: Vec<String>, failed: Vec<String>, like: bool) {
        // Any left (a retry took them) leave the list now.
        if !like {
            let rest: Vec<Track> = video_ids.iter().filter_map(|id| self.unliking.remove(id)).collect();
            self.set_liked(&rest, false);
            for id in &failed {
                self.unliking.remove(id);
            }
        }
        log::debug!("like={like} answered: {} taken, {} refused {:?}", video_ids.len(), failed.len(), failed.iter().take(5).collect::<Vec<_>>());
        self.lists.edits.confirm_likes(&video_ids, &failed);
        // The refused ones come back as they were with a reload.
        if !failed.is_empty() {
            self.request_liked(true);
        }
        let count = video_ids.len();
        let mut msg = format!("{} {count} {}", if like { "Liked" } else { "Removed like from" }, if count == 1 { "song" } else { "songs" });
        if count == 0 {
            msg = format!("YouTube Music refused all {} songs; try again in a while", failed.len());
        } else if !failed.is_empty() {
            msg += &format!("; YouTube Music refused {}", failed.len());
        }
        self.toast(msg);
    }

    /// A playlist was edited, added to, removed from, saved or deleted; `message` says which.
    fn on_playlist_changed(&mut self, id: String, message: &'static str) {
        self.request_library();
        // A sort taken waits on, for the reload in its order; anything else ends it.
        match self.sorting.get_mut(&id) {
            Some(w) if message == "Playlist sorted" => w.taken(),
            _ => {
                self.sorting.remove(&id);
            }
        }
        if message == "Playlist deleted" {
            // Its load is dropped too, and the queue stops waiting for it.
            self.lists.forget(&id);
            if self.queue.fill.as_ref().is_some_and(|f| f.id == id) {
                self.queue.fill = None;
            }
            let gone = Page::Playlist(id);
            if self.page == gone {
                self.go(Page::Library(LibTab::Playlists));
            }
            // Back and Forward skip it.
            self.back.retain(|p| *p != gone);
            self.back.dedup();
            self.forward.retain(|p| *p != gone);
            self.forward.dedup();
        } else {
            if message == "Removed from playlist" {
                self.lists.edits.confirm_removal(&id);
            }
            // Kept for going back (or playing): reload it. What YouTube Music still
            // lists after a removal is dropped as it arrives.
            if self.lists.known(&id) {
                self.load_list(&id, false, true);
            }
        }
        self.toast(message);
    }

    /// A song couldn't play: skip it, or stop when skipping can't help.
    fn on_play_error(&mut self, generation: u64, msg: String) {
        if generation == self.generation {
            self.buffering = false;
            if self.mix_from.take().is_some() {
                self.pending_mix = None;
                self.audio.send(AudioCmd::Stop);
                self.release_mix();
            }
            let title = self.current().map(|t| t.title.clone()).unwrap_or_default();
            let reason = if msg.contains("age-restricted") || msg.contains("confirm your age") {
                "it's age-restricted".to_owned()
            } else {
                msg
            };
            let has_next = !self.queue.at_end() && self.queue.index.is_some();
            // Skipping can't help when the account is the problem: the rest of
            // an album like this needs it too. Stop on the song instead.
            if let Some(why) = account_block(&reason, self.logged_in) {
                self.skip_streak = 0;
                self.toast(why.replace("{title}", &title));
            } else if has_next && self.skip_streak < MAX_SKIPS {
                // Don't stall the queue on one unplayable song.
                self.skip_streak += 1;
                self.toast(format!("Skipped \"{title}\": {reason}"));
                self.next();
            } else {
                self.skip_streak = 0;
                self.toast(format!("Can't play \"{title}\": {reason}"));
            }
        }
    }

    /// Signed in or out: what belonged to the last account goes, and the new one's home,
    /// library and likes load.
    fn on_logged_in(&mut self, v: bool, now: f64) {
        self.logged_in = v;
        self.channels = None;
        self.channels_asked = None;
        // Recent searches belong to the account.
        self.suggestions = Default::default();
        self.suggest_sel = None;
        self.forgetting = None;
        self.suggest_round += 1;
        self.suggest_cache.clear();
        self.forgets_pending = 0;
        self.suggest_due = Some(now);
        self.account_name = None;
        self.account_avatar = None;
        self.sidebar_cache = None;
        self.library = None;
        self.history = None;
        self.uploads = None;
        // Whether it's subscribed is this account's (or channel's) to say.
        self.artist = None;
        self.subscriptions = Default::default();
        // Your own playlists show differently (and private ones not at all) to another
        // account, and its likes aren't this one's.
        self.lists.clear();
        self.lists.edits = Default::default();
        self.liked.clear();
        self.unliking.clear();
        self.queue.fill = None;
        self.play_pending = None;
        if self.page_list().is_some() || matches!(self.page, Page::Artist(_)) {
            self.refresh();
        }
        self.home = None;
        self.home_loading = true;
        self.backend.request(Req::Home(None));
        if v {
            self.request_library();
            self.request_liked(false);
            self.backend.request(Req::Account);
        }
        self.toast(if v { "Signed in" } else { "Signed out" });
    }

    /// A request failed: pages waiting for it stop waiting, and the error shows.
    fn on_error(&mut self, e: String) {
        if let Some(Dialog::Collaborate { info, busy, .. }) = &mut self.dialog {
            *busy = false;
            // Nothing to show if it never loaded.
            if info.is_none() {
                self.dialog = None;
            }
        }
        // A page still waiting for its first data shows a retry instead of loading forever.
        self.load_failed |= self.loading || (self.page == Page::Home && self.home_loading);
        self.loading = false;
        self.home_loading = false;
        self.radio_pending = false;
        if let Some(h) = &mut self.home {
            h.loading_more = None;
            h.picks_loading = false;
        }
        self.toast(e);
    }
}

/// Unplayable songs skipped in a row before playback stops.
const MAX_SKIPS: u32 = 3;

/// When a song failed because of the account (Premium-only, or needs signing in),
/// the message to show instead of skipping: the rest of an album like that fails the
/// same way.
fn account_block(msg: &str, logged_in: bool) -> Option<&'static str> {
    let m = msg.to_ascii_lowercase();
    if m.contains("premium") {
        Some("\"{title}\" needs YouTube Music Premium")
    } else if !logged_in && (m.contains("sign in") || m.contains("age-restricted")) {
        Some("Sign in to play \"{title}\"")
    } else {
        None
    }
}

#[cfg(test)]
mod account_block_tests {
    use super::account_block;

    #[test]
    fn account_errors_stop_instead_of_skipping() {
        assert!(account_block("sign in to play this song", false).is_some());
        assert!(account_block("sign in to play this song", true).is_none());
        assert!(account_block("This video is only available to Music Premium members", true).is_some());
        assert!(account_block("decode: end of stream", false).is_none());
    }
}

fn songs(n: usize) -> String {
    if n == 1 { "1 song".to_owned() } else { format!("{n} songs") }
}

/// What a finished download of many songs did; `name` is " “its title”", or empty for a selection.
fn download_summary(r: &crate::backend::DownloadRun, name: &str) -> String {
    let mut msg = if r.cancelled {
        format!("Stopped downloading{name}: {} saved", songs(r.saved))
    } else if r.saved == 0 && r.failed.is_empty() {
        return if name.is_empty() { "Downloaded already".to_owned() } else { format!("{} is downloaded already", name.trim_start()) };
    } else {
        format!("Downloaded{name}: {}", songs(r.saved))
    };
    if r.skipped > 0 && !r.cancelled {
        msg += &format!(", {} already there", r.skipped);
    }
    if !r.failed.is_empty() {
        let few: Vec<&str> = r.failed.iter().take(3).map(String::as_str).collect();
        let more = if r.failed.len() > 3 { ", …" } else { "" };
        msg += &format!("; {} couldn't be downloaded ({}{more})", r.failed.len(), few.join(", "));
    }
    msg
}

#[cfg(test)]
mod download_summary_tests {
    use super::download_summary;
    use crate::backend::DownloadRun;

    fn run(saved: usize, skipped: usize, failed: &[&str], cancelled: bool) -> DownloadRun {
        let failed: Vec<String> = failed.iter().map(|s| s.to_string()).collect();
        DownloadRun { key: "PL1".into(), title: "Mix".into(), total: saved + failed.len(), done: saved + failed.len(), saved, skipped, failed, cancelled, ..Default::default() }
    }

    #[test]
    fn says_what_a_run_did() {
        let name = " \u{201c}Mix\u{201d}";
        assert_eq!(download_summary(&run(37, 3, &[], false), name), "Downloaded \u{201c}Mix\u{201d}: 37 songs, 3 already there");
        assert_eq!(download_summary(&run(0, 12, &[], false), name), "\u{201c}Mix\u{201d} is downloaded already");
        assert_eq!(download_summary(&run(0, 2, &[], false), ""), "Downloaded already");
        assert_eq!(download_summary(&run(5, 0, &[], true), name), "Stopped downloading \u{201c}Mix\u{201d}: 5 songs saved");
        assert_eq!(
            download_summary(&run(1, 0, &["A", "B", "C", "D"], false), ""),
            "Downloaded: 1 song; 4 couldn't be downloaded (A, B, C, …)"
        );
    }
}

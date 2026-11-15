//! System tray icon, always up from the first frame (ForkChord's tray: the song,
//! Open, transport buttons, Launch at login, Quit). Its Play/Pause label follows the
//! player like a real desktop player's would, the menu shows what's playing, and it
//! keeps working while the window is hidden behind it — the music goes on after a
//! close.
//!
//! The icon must be created on the UI thread (it owns a Win32 message window), so
//! the app creates and drops it from `logic`. Clicks arrive on that thread's message
//! loop; they are queued here and the UI is woken to handle them. On macOS it is a
//! menu bar item, made the same way (AppKit wants it on the main thread, which is the
//! UI thread there).
//!
//! On Linux the icon is a GTK one, and GTK runs its own loop on one thread for the
//! life of the process (it can't be started twice); the icon is made and dropped
//! there, and its menu items are touched only on that thread — the label updates
//! below hop over by way of the GTK main context.
//!
//! Note that on Windows the shell can accept the icon (so `Tray::new` succeeds) and
//! then never draw it — taskbar replacements and a busy notification-area cache can
//! both do that. Callers therefore re-create the icon every so often (see
//! `ui::integrations::tray_window`), and recovery never depends on the icon alone:
//! launching the app again (or `--show`) shows the window regardless.

use std::sync::{Mutex, Once};

use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cmd {
    Show,
    PlayPause,
    Next,
    Prev,
    Autostart,
    Quit,
}

static PENDING: Mutex<Vec<Cmd>> = Mutex::new(Vec::new());

pub struct Tray {
    #[cfg(not(target_os = "linux"))]
    icon: TrayIcon,
    /// The menu items that follow the player: the song, and Play/Pause.
    #[cfg(not(target_os = "linux"))]
    song: MenuItem,
    #[cfg(not(target_os = "linux"))]
    play: MenuItem,
    /// Next and Previous, with nothing to skip to until something plays.
    #[cfg(not(target_os = "linux"))]
    next: MenuItem,
    #[cfg(not(target_os = "linux"))]
    prev: MenuItem,
    /// The Launch at login item, so the tick answers right after a click on it.
    #[cfg(not(target_os = "linux"))]
    autostart: CheckMenuItem,
    /// The song the menu and tooltip show now, and whether the player says "Pause".
    now: Now,
}

/// The song the tray shows now, and whether it says "Pause".
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Now {
    /// "Title — Artist", or None when nothing's playing.
    song: Option<String>,
    playing: bool,
}

impl Tray {
    /// `wake` is called (from the UI thread's message loop) after each click.
    /// `playing` and `autostart_on` are what the menu shows until the next update.
    pub fn new(icon: &crate::appicon::Variant, playing: bool, autostart_on: bool, wake: impl Fn() + Send + Sync + Clone + 'static) -> anyhow::Result<Self> {
        // The handlers are process-wide and can only be set once.
        static HANDLERS: Once = Once::new();
        HANDLERS.call_once(|| {
            let w = wake.clone();
            TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
                if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
                    push(Cmd::Show, &w);
                }
            }));
            MenuEvent::set_event_handler(Some(move |e: MenuEvent| match e.id.as_ref() {
                "show" => push(Cmd::Show, &wake),
                "play-pause" => push(Cmd::PlayPause, &wake),
                "next" => push(Cmd::Next, &wake),
                "previous" => push(Cmd::Prev, &wake),
                "autostart" => push(Cmd::Autostart, &wake),
                "quit" => push(Cmd::Quit, &wake),
                _ => {}
            }));
        });

        let png = icon.rgba(32).ok_or_else(|| anyhow::anyhow!("icon doesn't decode"))?;
        #[cfg(not(target_os = "linux"))]
        return Self::non_linux(png, playing, autostart_on);
        #[cfg(target_os = "linux")]
        {
            gtk_thread::show(png, playing, autostart_on)?;
            Ok(Self { now: Now::default() })
        }
    }

    #[cfg(not(target_os = "linux"))]
    fn non_linux(png: image::RgbaImage, playing: bool, autostart_on: bool) -> anyhow::Result<Self> {
        let menu = TrayMenu::new(playing, autostart_on);
        let (w, h) = png.dimensions();
        let icon = TrayIconBuilder::new()
            .with_icon(tray_icon::Icon::from_rgba(png.into_raw(), w, h)?)
            .with_tooltip("BitChord")
            .with_menu(Box::new(menu.menu))
            .with_menu_on_left_click(false)
            .build()?;
        Ok(Self { icon, song: menu.song, play: menu.play, next: menu.next, prev: menu.prev, autostart: menu.autostart, now: Now::default() })
    }

    /// The song (title and artists) in the menu and the tooltip, and "Pause" while it plays.
    pub fn set_now(&mut self, song: Option<(&str, &str)>, playing: bool) {
        let song = song.map(|(title, artist)| if artist.is_empty() { title.to_owned() } else { format!("{title} — {artist}") });
        let now = Now { song, playing };
        if now == self.now {
            return;
        }
        self.now = now.clone();
        #[cfg(not(target_os = "linux"))]
        self.icon_menu_set(&now);
        #[cfg(target_os = "linux")]
        gtk_thread::set_now(now);
    }

    /// The menu items on the windowing system's own thread; the icon's tooltip with them.
    #[cfg(not(target_os = "linux"))]
    fn icon_menu_set(&self, now: &Now) {
        self.song.set_text(now.song.as_deref().unwrap_or("Nothing playing"));
        self.play.set_text(if now.playing { "Pause" } else { "Play" });
        // Nothing to play, skip or go back to.
        for item in [&self.play, &self.next, &self.prev] {
            item.set_enabled(now.song.is_some());
        }
        let _ = self.icon.set_tooltip(Some(tooltip(now.song.as_deref())));
    }

    /// Flips the Launch at login tick after the app toggled the OS state itself.
    #[cfg(not(target_os = "linux"))]
    pub fn set_autostart(&self, on: bool) {
        self.autostart.set_checked(on);
    }

    /// The menu item handles live on the GTK thread on Linux; these hop over.
    #[cfg(target_os = "linux")]
    pub fn set_autostart(&self, on: bool) {
        gtk_thread::set_autostart(on);
    }
}

/// "BitChord", with the song under it. Windows cuts a tooltip off at 127 characters
/// (UTF-16 ones), so a long one ends in "…" there instead.
fn tooltip(song: Option<&str>) -> String {
    const MAX: usize = 127;
    let Some(song) = song else { return "BitChord".to_owned() };
    let tip = format!("BitChord\n{song}");
    if tip.encode_utf16().count() <= MAX {
        return tip;
    }
    let mut out = String::new();
    let mut len = 1; // the "…"
    for c in tip.chars() {
        len += c.len_utf16();
        if len > MAX {
            break;
        }
        out.push(c);
    }
    out.push('…');
    out
}

/// The tray menu, ForkChord's: the song, Open, transport (Play/Pause named for the state
/// it would switch to, like every desktop player's), Launch at login, Quit.
struct TrayMenu {
    menu: Menu,
    song: MenuItem,
    play: MenuItem,
    next: MenuItem,
    prev: MenuItem,
    autostart: CheckMenuItem,
}

impl TrayMenu {
    fn new(playing: bool, autostart_on: bool) -> Self {
        let song = MenuItem::with_id("song", "Nothing playing", false, None);
        let open = MenuItem::with_id("show", "Open BitChord", true, None);
        let previous = MenuItem::with_id("previous", "Previous", false, None);
        let play = MenuItem::with_id("play-pause", if playing { "Pause" } else { "Play" }, false, None);
        let next = MenuItem::with_id("next", "Next", false, None);
        let autostart = CheckMenuItem::with_id("autostart", "Launch at login", true, autostart_on, None);
        let quit = MenuItem::with_id("quit", "Quit BitChord", true, None);
        let menu = Menu::with_items(&[
            &song,
            &PredefinedMenuItem::separator(),
            &open,
            &PredefinedMenuItem::separator(),
            &previous,
            &play,
            &next,
            &PredefinedMenuItem::separator(),
            &autostart,
            &PredefinedMenuItem::separator(),
            &quit,
        ])
        .expect("tray menu");
        Self { menu, song, play, next, prev: previous, autostart }
    }
}

#[cfg(target_os = "linux")]
impl Drop for Tray {
    fn drop(&mut self) {
        gtk_thread::hide();
    }
}

#[cfg(target_os = "linux")]
fn build(png: image::RgbaImage, playing: bool, autostart_on: bool) -> anyhow::Result<(TrayIcon, TrayMenuItems)> {
    let menu = TrayMenu::new(playing, autostart_on);
    let (w, h) = png.dimensions();
    let icon = TrayIconBuilder::new()
        .with_icon(tray_icon::Icon::from_rgba(png.into_raw(), w, h)?)
        .with_tooltip("BitChord")
        .with_menu(Box::new(menu.menu))
        .with_menu_on_left_click(false)
        .build()?;
    let items = TrayMenuItems { song: menu.song, play: menu.play, next: menu.next, prev: menu.prev, autostart: menu.autostart };
    Ok((icon, items))
}

/// The menu items the GTK thread keeps, so their labels can be set from it alone.
#[cfg(target_os = "linux")]
struct TrayMenuItems {
    song: MenuItem,
    play: MenuItem,
    next: MenuItem,
    prev: MenuItem,
    autostart: CheckMenuItem,
}

#[cfg(target_os = "linux")]
mod gtk_thread {
    use std::cell::RefCell;
    use std::sync::OnceLock;
    use std::time::Duration;

    use tray_icon::TrayIcon;

    thread_local! {
        static ICON: RefCell<Option<TrayIcon>> = const { RefCell::new(None) };
        /// The menu items that follow the player and the login setting; kept so
        /// their labels can be set from this thread alone.
        static LABELS: RefCell<Option<super::TrayMenuItems>> = const { RefCell::new(None) };
    }

    /// Start GTK's thread once; an error if GTK can't start (no display).
    fn start() -> anyhow::Result<()> {
        static STARTED: OnceLock<Result<(), String>> = OnceLock::new();
        STARTED
            .get_or_init(|| {
                let (tx, rx) = std::sync::mpsc::channel();
                std::thread::Builder::new()
                    .name("gtk".into())
                    .spawn(move || match gtk::init() {
                        Ok(()) => {
                            let _ = tx.send(Ok(()));
                            gtk::main();
                        }
                        Err(e) => {
                            let _ = tx.send(Err(e.to_string()));
                        }
                    })
                    .map_err(|e| e.to_string())?;
                rx.recv().map_err(|e| e.to_string())?
            })
            .clone()
            .map_err(|e| anyhow::anyhow!("GTK: {e}"))
    }

    pub fn show(png: image::RgbaImage, playing: bool, autostart_on: bool) -> anyhow::Result<()> {
        start()?;
        let (tx, rx) = std::sync::mpsc::channel();
        gtk::glib::MainContext::default().invoke(move || {
            let res = super::build(png, playing, autostart_on).map(|(icon, items)| {
                ICON.with(|i| *i.borrow_mut() = Some(icon));
                LABELS.with(|l| *l.borrow_mut() = Some(items));
            });
            let _ = tx.send(res.map_err(|e| e.to_string()));
        });
        rx.recv_timeout(Duration::from_secs(5))?.map_err(|e| anyhow::anyhow!(e))
    }

    pub fn hide() {
        gtk::glib::MainContext::default().invoke(|| ICON.with(|i| drop(i.borrow_mut().take())));
    }

    /// The song in the menu and the tooltip, and "Pause" while it plays, as the
    /// windowing system's own update does.
    pub fn set_now(now: super::Now) {
        gtk::glib::MainContext::default().invoke(move || {
            let tip = super::tooltip(now.song.as_deref());
            ICON.with(|i| {
                if let Some(icon) = &*i.borrow() {
                    let _ = icon.set_tooltip(Some(tip));
                }
            });
            LABELS.with(|l| {
                if let Some(items) = l.borrow().as_ref() {
                    items.song.set_text(now.song.as_deref().unwrap_or("Nothing playing"));
                    items.play.set_text(if now.playing { "Pause" } else { "Play" });
                    // Nothing to play, skip or go back to.
                    for item in [&items.play, &items.next, &items.prev] {
                        item.set_enabled(now.song.is_some());
                    }
                }
            });
        });
    }

    pub fn set_autostart(on: bool) {
        gtk::glib::MainContext::default().invoke(move || {
            LABELS.with(|l| {
                if let Some(items) = l.borrow().as_ref() {
                    items.autostart.set_checked(on);
                }
            });
        });
    }
}

fn push(cmd: Cmd, wake: &impl Fn()) {
    PENDING.lock().unwrap().push(cmd);
    wake();
}

/// Clicks since the last call.
pub fn take() -> Vec<Cmd> {
    std::mem::take(&mut *PENDING.lock().unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tooltip_fits_windows() {
        assert_eq!(tooltip(None), "BitChord");
        assert_eq!(tooltip(Some("Song — Artist")), "BitChord\nSong — Artist");
        // Just fits: left as it is.
        let fits = "a".repeat(127 - 9);
        assert_eq!(tooltip(Some(&fits)), format!("BitChord\n{fits}"));
        assert_eq!(tooltip(Some(&"a".repeat(200))), format!("BitChord\n{}…", "a".repeat(117)));
        // Cut between characters, never inside one (an emoji is two UTF-16 ones).
        let emoji = tooltip(Some(&"😀".repeat(100)));
        assert_eq!(emoji, format!("BitChord\n{}…", "😀".repeat(58)));
        assert!(emoji.encode_utf16().count() <= 127);
    }
}

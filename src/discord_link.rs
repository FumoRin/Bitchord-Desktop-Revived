//! Discord Rich Presence through the account itself ("Discord via link"): the
//! gateway is opened with the account's own token, so the card shows on the
//! user's real profile, with working buttons and artwork pulled in from any
//! https image. A port of ForkChord's `core/src/discord.rs` (which ports
//! `com.my.kizzy.gateway.DiscordWebSocket` and `DiscordRPC.updateSong`).
//!
//! This is user-token automation, as on the phone: there is no API for a
//! third-party app to set a user's presence, so the token is read from a
//! discord.com page in the sign-in window (login.rs, the fresh-iframe trick)
//! only when the user clicks Connect. The token lives in the vault, never in
//! the settings file.
//!
//! The card itself is the one `crate::discord` builds for the plain IPC path,
//! from the same Settings wording, so a linked listener's card is laid out the
//! same way — only through their own account, with the artwork mirrored onto
//! Discord's CDN. The plain IPC presence stays as the fallback for a listener
//! who doesn't link an account.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::sync::Mutex;
use tokio_tungstenite::tungstenite::Message;

/// BitChord's own Discord application (DiscordApplication.kt): what buttons and
/// the external-artwork mirror are filed under.
pub const APPLICATION_ID: &str = "1543679166967193620";
const GATEWAY: &str = "wss://gateway.discord.gg/?v=9&encoding=json";
const API: &str = "https://discord.com/api/v9";

const INITIAL_RECONNECT_MS: u64 = 1_000;
const MAX_RECONNECT_MS: u64 = 60_000;
/// A presence push waits this long for a session before it's sent anyway; it's
/// replayed onto the session once one arrives.
const SESSION_WAIT_MS: u64 = 20_000;
/// A cleared card with nothing new for this long: the socket goes too, so the
/// account isn't shown online by a music app that isn't playing anything.
const IDLE_CLOSE_MS: u64 = 5 * 60_000;

const OP_DISPATCH: u8 = 0;
const OP_HEARTBEAT: u8 = 1;
const OP_IDENTIFY: u8 = 2;
const OP_PRESENCE_UPDATE: u8 = 3;
const OP_RESUME: u8 = 6;
const OP_RECONNECT: u8 = 7;
const OP_INVALID_SESSION: u8 = 9;
const OP_HELLO: u8 = 10;
const OP_HEARTBEAT_ACK: u8 = 11;

// ---- How the card reads -------------------------------------------------------

/// The linked account, as Settings shows it (KizzyRPC.getUserInfo).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct User {
    pub name: String,
    pub username: String,
    pub avatar_url: Option<String>,
}

/// How the gateway is doing, for the Settings row: off | connecting | ready | error.
pub type Status = (&'static str, Option<String>);

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Which line the member list repeats, as Discord's number for it (the IPC payload's
/// `StatusDisplayType`, which the gateway takes too).
fn status_display_type(s: crate::discord::Status) -> u8 {
    match s {
        crate::discord::Status::Name => 0,
        crate::discord::Status::Line1 => 2,
        crate::discord::Status::Line2 => 1,
    }
}

/// Identify.toIdentifyPayload, with the web client's properties.
fn identify_payload(token: &str) -> Value {
    json!({
        "token": token,
        "capabilities": 16381,
        "properties": { "os": "Linux", "browser": "Chrome", "device": "" },
        "presence": { "status": "online", "since": 0, "activities": [], "afk": false },
        "compress": false,
        "client_state": {
            "guild_versions": {},
            "highest_last_message_id": "0",
            "read_state_version": 0,
            "user_guild_settings_version": -1,
            "user_settings_version": -1,
            "private_channels_version": "0",
            "api_code_version": 0,
        },
    })
}

fn heartbeat_frame(seq: u64) -> Value {
    json!({ "op": OP_HEARTBEAT, "d": if seq > 0 { Value::from(seq) } else { Value::Null } })
}

// ---- The gateway ---------------------------------------------------------------

enum Command {
    SetPresence(Value),
    ClearActivity,
}

#[derive(Default)]
struct Shared {
    sequence: u64,
    session_id: Option<String>,
    resume_url: Option<String>,
    /// Replayed onto every new session: Discord discards a presence with the
    /// session that set it.
    last_presence: Option<Value>,
    /// What this session already has, so the replay and a push don't both send it.
    delivered: Option<String>,
    ready: bool,
}

impl Shared {
    fn forget_session(&mut self) {
        self.resume_url = None;
        self.session_id = None;
        self.sequence = 0;
    }
}

fn rand_u64(min: u64, max: u64) -> u64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64)
        .unwrap_or(0);
    min + nanos % (max - min).max(1)
}

/// The gateway (DiscordWebSocket.kt): connect, pump, back off 1 s doubling to
/// 60 s, the heartbeat watchdog that drops a socket whose heartbeat went
/// unanswered, and the replay of the last presence onto a new session. The
/// socket lives on the backend's runtime; dropping the task closes it.
struct Gateway {
    shared: Arc<Mutex<Shared>>,
    commands: tokio::sync::mpsc::UnboundedSender<Command>,
    handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
}
enum Ended {
    /// Discord said the token is no good: stop trying, say so.
    Refused,
    /// Discord asked for an immediate reconnect (OP_RECONNECT).
    RetryNow,
    /// Anything else: back off and dial again.
    Backoff,
}

impl Gateway {
    fn spawn(
        backend: &crate::backend::Backend,
        token: String,
        status: Arc<dyn Fn(Status) + Send + Sync>,
    ) -> Arc<Gateway> {
        let shared = Arc::new(Mutex::new(Shared::default()));
        let (commands, command_rx) = tokio::sync::mpsc::unbounded_channel();
        let handle = backend.spawn_handle(Self::run(token, shared.clone(), command_rx, status));
        Arc::new(Gateway {
            shared,
            commands,
            handle: Mutex::new(Some(handle)),
        })
    }

    async fn is_ready(&self) -> bool {
        self.shared.lock().await.ready
    }

    fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }

    async fn close(&self) {
        if let Some(handle) = self.handle.lock().await.take() {
            handle.abort();
        }
        *self.shared.lock().await = Shared::default();
    }

    async fn run(
        token: String,
        shared: Arc<Mutex<Shared>>,
        mut commands: tokio::sync::mpsc::UnboundedReceiver<Command>,
        status: Arc<dyn Fn(Status) + Send + Sync>,
    ) {
        let report = |s: Status| status(s);
        let mut backoff = INITIAL_RECONNECT_MS;
        'outer: loop {
            report(("connecting", None));
            let url = shared
                .lock()
                .await
                .resume_url
                .clone()
                .unwrap_or_else(|| GATEWAY.to_string());
            // Through the proxy setting, like every other socket (see jam.rs).
            let connect = async {
                let u = url::Url::parse(&url)?;
                let host = u.host_str().unwrap_or_default().to_owned();
                let port = u.port_or_known_default().unwrap_or(443);
                let tcp = crate::proxy::connect(&host, port).await?;
                anyhow::Ok(tokio_tungstenite::client_async_tls(url, tcp).await?)
            };
            let socket = match tokio::time::timeout(Duration::from_secs(15), connect).await {
                Ok(Ok(pair)) => pair.0,
                Ok(Err(e)) => {
                    log::debug!("discord gateway: {e}");
                    tokio::time::sleep(Duration::from_millis(backoff)).await;
                    backoff = (backoff * 2).min(MAX_RECONNECT_MS);
                    continue;
                }
                Err(_) => {
                    tokio::time::sleep(Duration::from_millis(backoff)).await;
                    backoff = (backoff * 2).min(MAX_RECONNECT_MS);
                    continue;
                }
            };
            backoff = INITIAL_RECONNECT_MS;
            let (mut write, mut read) = socket.split();
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Message>();
            let writer = tokio::spawn(async move {
                while let Some(message) = rx.recv().await {
                    if write.send(message).await.is_err() {
                        break;
                    }
                }
            });

            let mut awaiting_ack = false;
            let mut heartbeat_interval: u64 = 0;
            let mut opened = false;
            let mut ended = Ended::Backoff;
            // Discord asks for the first heartbeat after a random fraction of the interval.
            let mut next_beat = tokio::time::Instant::now() + Duration::from_millis(1_000);

            loop {
                tokio::select! {
                    command = commands.recv() => {
                        let Some(command) = command else { break };
                        let presence = match command {
                            Command::SetPresence(presence) => Some(presence),
                            Command::ClearActivity => Some(json!({ "activities": [], "afk": true, "since": 0, "status": "online" })),
                        };
                        let presence = presence.expect("both arms produce one");
                        let mut guard = shared.lock().await;
                        guard.last_presence = Some(presence.clone());
                        if guard.ready {
                            let key = presence.to_string();
                            if guard.delivered.as_deref() != Some(key.as_str()) {
                                let frame = Message::text(json!({ "op": OP_PRESENCE_UPDATE, "d": presence }).to_string());
                                if tx.send(frame).is_ok() {
                                    guard.delivered = Some(key);
                                }
                            }
                        }
                    }
                    frame = read.next() => {
                        let Some(frame) = frame else { break };
                        let Ok(frame) = frame else { break };
                        match frame {
                            Message::Close(close) => {
                                // 4004: authentication failed. Retrying with the same token never helps.
                                if close.as_ref().map(|f| u16::from(f.code)) == Some(4004) {
                                    ended = Ended::Refused;
                                }
                                break;
                            }
                            Message::Text(text) => {
                                let Ok(payload) = serde_json::from_str::<Value>(&text) else { continue };
                                if let Some(s) = payload.get("s").and_then(|s| s.as_u64()) {
                                    shared.lock().await.sequence = s;
                                }
                                let op = payload.get("op").and_then(|o| o.as_u64()).unwrap_or(u64::MAX) as u8;
                                match op {
                                    OP_DISPATCH => {
                                        if payload.get("t").and_then(|t| t.as_str()) == Some("READY") {
                                            let d = payload.get("d").cloned().unwrap_or(Value::Null);
                                            let mut guard = shared.lock().await;
                                            guard.session_id = d.get("session_id").and_then(|s| s.as_str()).map(|s| s.to_string());
                                            guard.resume_url = d.get("resume_gateway_url").and_then(|r| r.as_str()).map(|r| format!("{r}/?v=9&encoding=json"));
                                            guard.ready = true;
                                            drop(guard);
                                            opened = true;
                                            awaiting_ack = false;
                                            report(("ready", None));
                                            let replay = shared.lock().await.last_presence.clone();
                                            if let Some(presence) = replay {
                                                let key = presence.to_string();
                                                let frame = Message::text(json!({ "op": OP_PRESENCE_UPDATE, "d": presence }).to_string());
                                                if tx.send(frame).is_ok() {
                                                    shared.lock().await.delivered = Some(key);
                                                }
                                            }
                                        }
                                    }
                                    OP_HELLO => {
                                        heartbeat_interval = payload.pointer("/d/heartbeat_interval").and_then(|i| i.as_u64()).unwrap_or(41_250);
                                        let (seq, session) = {
                                            let guard = shared.lock().await;
                                            (guard.sequence, guard.session_id.clone())
                                        };
                                        let out = if seq > 0 {
                                            json!({ "op": OP_RESUME, "d": { "token": token, "session_id": session, "seq": seq } })
                                        } else {
                                            json!({ "op": OP_IDENTIFY, "d": identify_payload(&token) })
                                        };
                                        let _ = tx.send(Message::text(out.to_string()));
                                        next_beat = tokio::time::Instant::now() + Duration::from_millis(heartbeat_interval * (rand_u64(1000, 2000) / 1000).max(1));
                                    }
                                    OP_HEARTBEAT => {
                                        let seq = shared.lock().await.sequence;
                                        let _ = tx.send(Message::text(heartbeat_frame(seq).to_string()));
                                    }
                                    OP_HEARTBEAT_ACK => {
                                        awaiting_ack = false;
                                    }
                                    OP_RECONNECT => {
                                        ended = Ended::RetryNow;
                                        break;
                                    }
                                    OP_INVALID_SESSION => {
                                        shared.lock().await.forget_session();
                                        let _ = tx.send(Message::text(json!({ "op": OP_IDENTIFY, "d": identify_payload(&token) }).to_string()));
                                    }
                                    _ => {}
                                }
                            }
                            _ => {}
                        }
                    }
                    _ = tokio::time::sleep_until(next_beat) => {
                        // The watchdog: a heartbeat unanswered for a whole interval means the socket is gone.
                        if opened {
                            if awaiting_ack {
                                ended = Ended::Backoff;
                                break;
                            }
                            awaiting_ack = true;
                            let seq = shared.lock().await.sequence;
                            let _ = tx.send(Message::text(heartbeat_frame(seq).to_string()));
                        }
                        next_beat = tokio::time::Instant::now() + Duration::from_millis(if heartbeat_interval > 0 { heartbeat_interval } else { 41_250 });
                    }
                }
            }

            writer.abort();
            {
                let mut guard = shared.lock().await;
                guard.ready = false;
                guard.delivered = None;
                // An attempt that never opened is as often a stale resume URL as a dead network.
                if !opened {
                    guard.forget_session();
                }
            }
            match ended {
                Ended::Refused => {
                    report((
                        "error",
                        Some("Discord refused the token. Connect the account again.".into()),
                    ));
                    return;
                }
                Ended::RetryNow => continue,
                Ended::Backoff => {
                    report(("connecting", None));
                    tokio::time::sleep(Duration::from_millis(backoff)).await;
                    backoff = (backoff * 2).min(MAX_RECONNECT_MS);
                    continue 'outer;
                }
            }
        }
    }
}

// ---- The service ---------------------------------------------------------------

/// One linked Discord account's presence. The gateway is brought up for the
/// first card, kept while music plays, and dropped a while after the last
/// cleared one (an idle app shouldn't hold the account online).
pub struct Service {
    backend: crate::backend::Backend,
    http: reqwest::Client,
    gateway: Arc<Mutex<Option<Arc<Gateway>>>>,
    status: Arc<std::sync::Mutex<Status>>,
    /// Holds the idle close so a new activity can cancel it.
    idle: Arc<Mutex<Option<tokio::task::AbortHandle>>>,
    /// Discord's CDN mirror of an https image, as an `mp:` asset (ExternalAssets.kt),
    /// cached: the same cover is asked for on every card.
    artwork: Arc<Mutex<HashMap<String, Option<String>>>>,
}

impl Service {
    pub fn new(backend: &crate::backend::Backend) -> Self {
        let http = crate::proxy::apply(reqwest::Client::builder())
            .user_agent("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36")
            .build()
            .expect("discord http client");
        Self {
            backend: backend.clone(),
            http,
            gateway: Arc::new(Mutex::new(None)),
            status: Arc::new(std::sync::Mutex::new(("off", None))),
            idle: Arc::new(Mutex::new(None)),
            artwork: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn status(&self) -> Status {
        self.status.lock().unwrap().clone()
    }

    /// ExternalAssets.kt: Discord's CDN mirror of an http image, as an `mp:` asset;
    /// cached (ArtworkCache). A mirror that wouldn't come keeps the raw URL, as
    /// RpcImage.ExternalImage does.
    async fn external_asset(&self, token: &str, image_url: &str) -> Option<String> {
        if let Some(cached) = self.artwork.lock().await.get(image_url).cloned() {
            return cached;
        }
        let result = async {
            let response = self
                .http
                .post(format!(
                    "{API}/applications/{APPLICATION_ID}/external-assets"
                ))
                .header("Authorization", token)
                .json(&json!({ "urls": [image_url] }))
                .send()
                .await
                .ok()?
                .error_for_status()
                .ok()?
                .json::<Value>()
                .await
                .ok()?;
            let path = response
                .as_array()?
                .first()?
                .get("external_asset_path")?
                .as_str()?
                .to_string();
            Some(format!("mp:{path}"))
        }
        .await;
        let mut cache = self.artwork.lock().await;
        if cache.len() > 50 {
            cache.clear();
        }
        cache.insert(image_url.to_string(), result.clone());
        result
    }

    /// The card as the gateway takes it: the same fields the IPC payload carries
    /// (name, the two lines and their links, the assets with the hover text, the
    /// timestamps, the buttons), with the artwork mirrored onto Discord's CDN as an
    /// `mp:` asset. `DiscordRPC.updateSong`'s shape.
    async fn build_presence(&self, token: &str, c: &crate::discord::Card) -> Value {
        let now = now_ms();
        // "Listening": the verb the card carries, as the IPC one's does.
        let mut activity = json!({
            "name": crate::discord::field(c.name()),
            "type": 2,
            "status_display_type": status_display_type(c.status),
            "url": Value::Null,
        });
        let object = activity.as_object_mut().unwrap();
        if let Some(l) = &c.lines[0] {
            object.insert("details".into(), json!(crate::discord::field(&l.text)));
            object.insert("details_url".into(), l.url.clone().map(Value::String).unwrap_or(Value::Null));
        }
        if let Some(l) = &c.lines[1] {
            object.insert("state".into(), json!(crate::discord::field(&l.text)));
            object.insert("state_url".into(), l.url.clone().map(Value::String).unwrap_or(Value::Null));
        }
        // The clock the card's bar runs on: real time, the listening rate already
        // divided out where the card was built.
        if let Some((start, end)) = c.times {
            object.insert("timestamps".into(), json!({ "start": start, "end": end }));
        }
        let art = c.art.as_deref().filter(|u| u.starts_with("https://") || u.starts_with("http://"));
        let large_image = match art {
            Some(url) => Some(
                self.external_asset(token, url)
                    .await
                    .unwrap_or_else(|| url.to_string()),
            ),
            None => None,
        };
        let mut assets = json!({
            "large_image": large_image,
            "large_text": c.lines[2].as_ref().map(|l| json!(crate::discord::field(&l.text))).unwrap_or(Value::Null),
        });
        if let Some(u) = &c.art_url {
            assets.as_object_mut().unwrap().insert("large_url".into(), json!(u));
        }
        object.insert("assets".into(), assets);
        if !c.buttons.is_empty() {
            object.insert(
                "buttons".into(),
                Value::Array(c.buttons.iter().map(|(label, _)| Value::String((*label).to_owned())).collect()),
            );
            object.insert("metadata".into(), json!({ "button_urls": c.buttons.iter().map(|(_, url)| Value::String(url.clone())).collect::<Vec<_>>() }));
            object.insert(
                "application_id".into(),
                Value::String(APPLICATION_ID.into()),
            );
        }
        json!({
            "activities": [activity],
            "afk": true,
            "since": now,
            // The account's own presence status is none of a music player's business;
            // "online" is what Discord itself keeps while its desktop app is open.
            "status": "online",
        })
    }

    /// The page reports what's playing; `None` takes the card down but leaves the
    /// socket up for a while (an idle close puts the socket away later, so an idle
    /// app doesn't hold the account online).
    pub fn set_presence(self: &Arc<Self>, token: String, card: Option<crate::discord::Card>) {
        let this = self.clone();
        self.backend.spawn(async move {
            // Cancel any pending idle close.
            if let Some(handle) = this.idle.lock().await.take() {
                handle.abort();
            }
            let Some(card) = card else {
                if let Some(gateway) = this.gateway.lock().await.clone() {
                    gateway.send(Command::ClearActivity);
                }
                let idle = this.clone();
                let handle = this.backend.spawn_handle(async move {
                    tokio::time::sleep(Duration::from_millis(IDLE_CLOSE_MS)).await;
                    idle.close_gateway().await;
                });
                *this.idle.lock().await = Some(handle.abort_handle());
                return;
            };
            let gateway = {
                let mut guard = this.gateway.lock().await;
                match guard.as_ref() {
                    Some(existing) => existing.clone(),
                    None => {
                        let backend = this.backend.clone();
                        let status_shared = this.status.clone();
                        let status: Arc<dyn Fn(Status) + Send + Sync> =
                            Arc::new(move |s: Status| {
                                *status_shared.lock().unwrap() = s;
                                backend.wake();
                            });
                        let gateway = Gateway::spawn(&this.backend, token.clone(), status);
                        *guard = Some(gateway.clone());
                        gateway
                    }
                }
            };
            gateway.send(Command::SetPresence(
                this.build_presence(&token, &card).await,
            ));
            // Wait a while for a session; a push that times out is replayed when one arrives.
            let deadline = tokio::time::Instant::now() + Duration::from_millis(SESSION_WAIT_MS);
            while tokio::time::Instant::now() < deadline {
                if gateway.is_ready().await {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
            // Sent again on the session that answered: the first one's artwork mirror
            // warmed the cache, so this is the one that carries it.
            gateway.send(Command::SetPresence(
                this.build_presence(&token, &card).await,
            ));
        });
    }

    /// Drop the socket (and forget it): the next card brings it back up.
    pub fn shutdown(self: &Arc<Self>) {
        let this = self.clone();
        self.backend.spawn(async move {
            this.close_gateway().await;
        });
    }

    /// Drop the socket (and forget it): the next card brings it back up.
    async fn close_gateway(&self) {
        if let Some(handle) = self.idle.lock().await.take() {
            handle.abort();
        }
        if let Some(gateway) = self.gateway.lock().await.take() {
            gateway.close().await;
        }
        *self.status.lock().unwrap() = ("off", None);
        self.artwork.lock().await.clear();
        self.backend.wake();
    }

    /// Where the linked account's gateway stands.
    pub async fn user_info(&self, token: &str) -> anyhow::Result<User> {
        let response = self
            .http
            .get(format!("{API}/users/@me"))
            .header("Authorization", token)
            .send()
            .await?;
        if response.status().as_u16() == 401 {
            anyhow::bail!("Discord refused the token");
        }
        let me: Value = response.json().await?;
        let id = me
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let username = me
            .get("username")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let name = me
            .get("global_name")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or(&username)
            .to_string();
        let avatar_url = me
            .get("avatar")
            .and_then(|v| v.as_str())
            .map(|avatar| format!("https://cdn.discordapp.com/avatars/{id}/{avatar}.png"));
        Ok(User {
            name,
            username,
            avatar_url,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_member_list_line_is_discords_number_for_it() {
        assert_eq!(status_display_type(crate::discord::Status::Name), 0);
        assert_eq!(status_display_type(crate::discord::Status::Line1), 2);
        assert_eq!(status_display_type(crate::discord::Status::Line2), 1);
    }

    #[test]
    fn identify_carries_the_web_properties() {
        let payload = identify_payload("tok");
        assert_eq!(payload["token"], "tok");
        assert_eq!(payload["properties"]["browser"], "Chrome");
        assert_eq!(payload["client_state"]["highest_last_message_id"], "0");
    }

    #[test]
    fn heartbeat_reports_the_sequence() {
        assert_eq!(heartbeat_frame(0)["d"], Value::Null);
        assert_eq!(heartbeat_frame(42)["d"], 42);
    }
}

//! Mode OBS: server HTTP kecil di localhost yang menyajikan chat sebagai halaman
//! HTML untuk Browser Source OBS Studio.
//!
//! Data chat sama dengan overlay di layar (satu poller YouTube), jadi gaya,
//! Super Chat, emoji, dan avatar otomatis konsisten. Server hanya mendengarkan
//! di 127.0.0.1 dan bisa dinyalakan/dimatikan dari panel.

use crate::{
    config::Config,
    model::{ChatMsg, Event, Kind},
    overlay::css_color,
};
use serde_json::json;
use std::{
    collections::{hash_map::DefaultHasher, HashMap, VecDeque},
    hash::{Hash, Hasher},
    net::Ipv4Addr,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, MutexGuard,
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

pub const DEFAULT_PORT: u16 = 8765;

/// Jumlah pesan terakhir yang dikirim ulang ke klien baru (mis. OBS baru dibuka).
const HISTORY_MAX: usize = 60;
/// Detak pemeriksaan: koneksi SSE ikut berhenti saat server dimatikan.
const TICK: Duration = Duration::from_secs(1);
const PING_EVERY_TICKS: u32 = 15;
const MAX_HEAD: usize = 16 * 1024;

const INDEX_HTML: &str = include_str!("../assets/obs/index.html");
const APP_JS: &str = include_str!("../assets/obs/app.js");

// ---------------------------------------------------------------- tampilan

/// Potongan config yang memengaruhi tampilan halaman OBS.
#[derive(Clone, Debug, PartialEq)]
pub struct Style {
    bg_opacity: f64,
    font_family: String,
    font_size: f64,
    text_color: String,
    avatar_px: i32,
    msg_gap: i32,
    show_avatars: bool,
    fade_after: f64,
    fade_duration: f64,
    max_messages: usize,
}

impl Style {
    fn from_config(cfg: &Config) -> Style {
        Style {
            bg_opacity: cfg.bg_opacity,
            font_family: cfg.font_family.clone(),
            font_size: cfg.font_size,
            text_color: cfg.text_color.clone(),
            avatar_px: cfg.avatar_px,
            msg_gap: cfg.msg_gap,
            show_avatars: cfg.show_avatars,
            fade_after: cfg.fade_after_secs,
            fade_duration: cfg.fade_duration_secs,
            max_messages: cfg.max_messages.max(1),
        }
    }

    /// CSS halaman OBS; sengaja mengikuti nilai yang sama dengan overlay GTK.
    fn css(&self) -> String {
        let bg = self.bg_opacity.clamp(0.0, 1.0);
        let fg = css_color(&self.text_color);
        let font = {
            let f = self.font_family.replace(['"', '\\'], "");
            if f.trim().is_empty() {
                "sans-serif".to_string()
            } else {
                f
            }
        };
        let px = self.font_size.clamp(8.0, 72.0);
        let author_px = (px * 0.8).max(7.0);
        let radius = (px * 0.5).round();
        let gap = self.msg_gap.clamp(0, 20);
        let av = self.avatar_px.clamp(12, 48);
        let fade = self.fade_duration.max(0.1);
        let avatars = if self.show_avatars {
            "inline-block"
        } else {
            "none"
        };

        format!(
            r#"
html, body {{
    margin: 0;
    padding: 0;
    height: 100%;
    background: transparent;
    overflow: hidden;
}}
#stage {{
    position: absolute;
    left: 0;
    right: 0;
    bottom: 0;
    display: flex;
    flex-direction: column;
    gap: {gap}px;
    padding: 4px;
    color: {fg};
    font-family: "{font}", sans-serif;
    font-size: {px:.2}px;
    line-height: 1.3;
    text-shadow: 0 1px 3px rgba(0, 0, 0, 0.98), 0 0 6px rgba(0, 0, 0, 0.9);
}}
.msg {{
    display: flex;
    align-items: flex-start;
    gap: 5px;
    background-color: rgba(0, 0, 0, {bg:.3});
    border-radius: {radius}px;
    padding: 2px 7px;
    transition: opacity {fade}s linear;
}}
.msg .col {{
    display: flex;
    flex-direction: column;
    gap: 1px;
    flex: 1;
    min-width: 0;
}}
.msg .head {{
    display: flex;
    align-items: baseline;
    gap: 4px;
}}
.msg .author {{
    font-weight: 700;
    font-size: {author_px:.2}px;
    color: #c7d2fe;
}}
.msg .amount {{
    font-weight: 700;
    font-size: {author_px:.2}px;
    color: #ffffff;
}}
.msg .body {{
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    color: {fg};
}}
.msg .emoji {{
    height: 1.25em;
    vertical-align: -0.25em;
}}
.msg .avatar {{
    display: {avatars};
    width: {av}px;
    height: {av}px;
    flex: none;
    border-radius: 50%;
    object-fit: cover;
}}
.msg.owner .author {{ color: #ffd54f; }}
.msg.mod .author {{ color: #8ab4f8; }}
.msg.member {{ border-left: 3px solid rgba(43, 166, 64, 0.9); }}
.msg.member .author {{ color: #7ee08a; }}
.msg.system {{ background-color: rgba(0, 0, 0, 0.55); }}
.msg.system .body {{ color: #dfe6ee; font-style: italic; }}
.msg.sc1 {{ background-color: rgba(30, 136, 229, 0.92); }}
.msg.sc2 {{ background-color: rgba(0, 184, 212, 0.92); }}
.msg.sc3 {{ background-color: rgba(255, 179, 0, 0.92); }}
.msg.sc4 {{ background-color: rgba(245, 124, 0, 0.92); }}
.msg.sc5 {{ background-color: rgba(233, 30, 99, 0.92); }}
.msg.sc6, .msg.sc7 {{ background-color: rgba(208, 0, 0, 0.92); }}
.msg.sc1 .body, .msg.sc2 .body, .msg.sc4 .body,
.msg.sc5 .body, .msg.sc6 .body, .msg.sc7 .body {{ color: #ffffff; }}
.msg.sc3 .body {{ color: #1b1b1b; }}
.msg.sc1 .author, .msg.sc2 .author, .msg.sc4 .author,
.msg.sc5 .author, .msg.sc6 .author, .msg.sc7 .author {{ color: #ffffff; }}
.msg.sc3 .author {{ color: #1b1b1b; }}
.msg.sc1, .msg.sc2, .msg.sc3, .msg.sc4,
.msg.sc5, .msg.sc6, .msg.sc7 {{ text-shadow: none; }}
.msg.fading {{ opacity: 0; }}
"#
        )
    }
}

// -------------------------------------------------------------------- hub

/// Penampung bersama: klien SSE, riwayat pesan, cache gambar, dan gaya aktif.
#[derive(Clone)]
pub struct Hub {
    inner: Arc<Mutex<Inner>>,
}

struct Inner {
    clients: Vec<async_channel::Sender<Arc<str>>>,
    history: VecDeque<Arc<str>>,
    images: HashMap<String, Image>,
    style: Style,
}

struct Image {
    bytes: Vec<u8>,
    mime: &'static str,
}

impl Hub {
    pub fn new(cfg: &Config) -> Hub {
        Hub {
            inner: Arc::new(Mutex::new(Inner {
                clients: Vec::new(),
                history: VecDeque::new(),
                images: HashMap::new(),
                style: Style::from_config(cfg),
            })),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Kirim perubahan tampilan ke klien yang sedang terhubung.
    pub fn set_style(&self, cfg: &Config) {
        let style = Style::from_config(cfg);
        let mut inner = self.lock();
        if inner.style == style {
            return;
        }
        inner.style = style;
        let frame = style_frame(&inner.style);
        broadcast(&mut inner, frame);
    }

    /// Teruskan satu event aplikasi ke halaman OBS.
    pub fn publish(&self, ev: &Event) {
        match ev {
            Event::Chat(msg) => {
                let frame = self.chat_frame(msg);
                self.push(frame, true);
            }
            Event::Status(text) => {
                let frame = self.chat_frame(&ChatMsg::system(text.clone()));
                self.push(frame, true);
            }
            Event::Ctl(crate::model::Ctl::Clear) => {
                self.push(Arc::from(r#"{"type":"clear"}"#), false);
            }
            Event::Ctl(_) => {}
        }
    }

    fn push(&self, frame: Arc<str>, remember: bool) {
        let mut inner = self.lock();
        if remember {
            inner.history.push_back(frame.clone());
            while inner.history.len() > HISTORY_MAX {
                inner.history.pop_front();
            }
        }
        broadcast(&mut inner, frame);
    }

    fn chat_frame(&self, msg: &ChatMsg) -> Arc<str> {
        let avatar = msg
            .avatar
            .as_ref()
            .map(|bytes| format!("/img/{}", self.image(bytes)));

        let mut emotes = serde_json::Map::new();
        for emote in &msg.emotes {
            if !emotes.contains_key(&emote.name) {
                let url = format!("/img/{}", self.image(emote.png.as_slice()));
                emotes.insert(emote.name.clone(), json!(url));
            }
        }

        let (kind, tier, amount) = match &msg.kind {
            Kind::Text => ("text", 0, String::new()),
            Kind::SuperChat { amount, tier } => ("superchat", *tier, amount.clone()),
            Kind::SuperSticker { amount, tier } => ("sticker", *tier, amount.clone()),
            Kind::Member => ("member", 0, String::new()),
            Kind::System => ("system", 0, String::new()),
        };

        Arc::from(
            json!({
                "type": "msg",
                "author": msg.author,
                "text": msg.text,
                "avatar": avatar,
                "kind": kind,
                "tier": tier,
                "amount": amount,
                "owner": msg.is_owner,
                "mod": msg.is_mod,
                "member": msg.is_member,
                "verified": msg.is_verified,
                "emotes": emotes,
            })
            .to_string(),
        )
    }

    /// Simpan gambar (avatar/emoji) di memori dan kembalikan kunci URL-nya.
    fn image(&self, bytes: &[u8]) -> String {
        let mut hasher = DefaultHasher::new();
        bytes.hash(&mut hasher);
        let key = format!("{:x}-{}", hasher.finish(), bytes.len());

        let mut inner = self.lock();
        inner.images.entry(key.clone()).or_insert_with(|| Image {
            bytes: bytes.to_vec(),
            mime: sniff_mime(bytes),
        });
        key
    }

    fn snapshot(&self) -> (Arc<str>, Vec<Arc<str>>, async_channel::Receiver<Arc<str>>) {
        let (tx, rx) = async_channel::unbounded();
        let mut inner = self.lock();
        let style = style_frame(&inner.style);
        let history: Vec<Arc<str>> = inner.history.iter().cloned().collect();
        inner.clients.push(tx);
        (style, history, rx)
    }

    fn image_by_key(&self, key: &str) -> Option<(Vec<u8>, &'static str)> {
        let inner = self.lock();
        inner
            .images
            .get(key)
            .map(|img| (img.bytes.clone(), img.mime))
    }
}

fn broadcast(inner: &mut Inner, frame: Arc<str>) {
    // Klien yang soketnya sudah tutup akan gagal menerima, jadi dibuang di sini.
    inner
        .clients
        .retain(|client| client.try_send(frame.clone()).is_ok());
}

fn style_frame(style: &Style) -> Arc<str> {
    Arc::from(
        json!({
            "type": "style",
            "css": style.css(),
            "fadeAfter": style.fade_after,
            "fadeDuration": style.fade_duration,
            "maxMessages": style.max_messages,
        })
        .to_string(),
    )
}

fn sniff_mime(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        "image/png"
    } else if bytes.starts_with(&[0xFF, 0xD8]) {
        "image/jpeg"
    } else if bytes.starts_with(b"GIF8") {
        "image/gif"
    } else if bytes.len() > 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        "image/webp"
    } else {
        "application/octet-stream"
    }
}

// ------------------------------------------------------- kontrol dari panel

#[derive(Clone, Debug, Default)]
pub struct ObsState {
    pub running: bool,
    pub error: Option<String>,
}

enum ObsCmd {
    Start,
    Stop,
}

/// Pegangan panel untuk menyalakan/mematikan server OBS.
#[derive(Clone)]
pub struct ObsCtl {
    cmd: async_channel::Sender<ObsCmd>,
    state: Arc<Mutex<ObsState>>,
    port: u16,
}

impl ObsCtl {
    pub fn spawn(hub: Hub, port: u16) -> ObsCtl {
        let (cmd, cmd_rx) = async_channel::unbounded::<ObsCmd>();
        let state = Arc::new(Mutex::new(ObsState::default()));
        let state_bg = state.clone();

        let spawned = std::thread::Builder::new()
            .name("obs-server".into())
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        set_error(&state_bg, format!("Gagal membuat runtime: {e}"));
                        return;
                    }
                };

                rt.block_on(async move {
                    let mut server: Option<(async_channel::Sender<()>, tokio::task::JoinHandle<()>)> =
                        None;

                    while let Ok(command) = cmd_rx.recv().await {
                        match command {
                            ObsCmd::Start => {
                                if server.is_some() {
                                    continue;
                                }
                                match TcpListener::bind((Ipv4Addr::LOCALHOST, port)).await {
                                    Ok(listener) => {
                                        {
                                            let mut st = lock_state(&state_bg);
                                            st.running = true;
                                            st.error = None;
                                        }
                                        let (stop_tx, stop_rx) = async_channel::unbounded::<()>();
                                        let hub = hub.clone();
                                        let state = state_bg.clone();
                                        let alive = Arc::new(AtomicBool::new(true));
                                        let handle = tokio::spawn(async move {
                                            accept_loop(listener, hub, stop_rx, alive).await;
                                            let mut st = lock_state(&state);
                                            st.running = false;
                                        });
                                        server = Some((stop_tx, handle));
                                    }
                                    Err(e) => set_error(
                                        &state_bg,
                                        format!("Port {port} tidak bisa dipakai: {e}"),
                                    ),
                                }
                            }
                            ObsCmd::Stop => {
                                if let Some((stop, handle)) = server.take() {
                                    let _ = stop.send(()).await;
                                    // Tunggu sampai listener benar-benar ditutup supaya
                                    // port langsung bebas dipakai lagi.
                                    let _ = handle.await;
                                }
                                set_running(&state_bg, false);
                            }
                        }
                    }
                });
            });

        if let Err(e) = spawned {
            set_error(&state, format!("Gagal membuat thread server: {e}"));
        }

        ObsCtl { cmd, state, port }
    }

    pub fn start(&self) {
        let _ = self.cmd.try_send(ObsCmd::Start);
    }

    pub fn stop(&self) {
        let _ = self.cmd.try_send(ObsCmd::Stop);
    }

    pub fn state(&self) -> ObsState {
        lock_state(&self.state).clone()
    }

    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}/", self.port)
    }
}

fn lock_state(state: &Arc<Mutex<ObsState>>) -> MutexGuard<'_, ObsState> {
    state.lock().unwrap_or_else(|e| e.into_inner())
}

fn set_running(state: &Arc<Mutex<ObsState>>, running: bool) {
    let mut st = lock_state(state);
    st.running = running;
    if running {
        st.error = None;
    }
}

fn set_error(state: &Arc<Mutex<ObsState>>, message: String) {
    let mut st = lock_state(state);
    st.running = false;
    st.error = Some(message);
}

// ------------------------------------------------------------------ server

async fn accept_loop(
    listener: TcpListener,
    hub: Hub,
    stop: async_channel::Receiver<()>,
    alive: Arc<AtomicBool>,
) {
    loop {
        tokio::select! {
            _ = stop.recv() => break,
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => {
                    let hub = hub.clone();
                    let alive = alive.clone();
                    tokio::spawn(handle_connection(stream, hub, alive));
                }
                Err(_) => break,
            }
        }
    }
    // Koneksi SSE yang masih terbuka ikut ditutup dalam ±1 detik.
    alive.store(false, Ordering::Relaxed);
}

async fn handle_connection(mut stream: TcpStream, hub: Hub, alive: Arc<AtomicBool>) {
    let Some((method, path)) = read_request(&mut stream).await else {
        return;
    };
    let head_only = method.eq_ignore_ascii_case("HEAD");

    if path == "/events" {
        stream_events(stream, hub, alive, head_only).await;
        return;
    }

    let (status, mime, body, cache) = if path == "/" || path == "/index.html" {
        ("200 OK", "text/html; charset=utf-8", INDEX_HTML.as_bytes().to_vec(), "no-store")
    } else if path == "/app.js" {
        (
            "200 OK",
            "text/javascript; charset=utf-8",
            APP_JS.as_bytes().to_vec(),
            "no-store",
        )
    } else if path == "/style.css" {
        (
            "200 OK",
            "text/css; charset=utf-8",
            hub.lock().style.css().into_bytes(),
            "no-store",
        )
    } else if let Some(key) = path.strip_prefix("/img/") {
        match hub.image_by_key(key) {
            Some((bytes, mime)) => ("200 OK", mime, bytes, "public, max-age=86400"),
            None => ("404 Not Found", "text/plain; charset=utf-8", b"Gambar tidak ada".to_vec(), "no-store"),
        }
    } else {
        (
            "404 Not Found",
            "text/plain; charset=utf-8",
            b"Tidak ditemukan".to_vec(),
            "no-store",
        )
    };

    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\n\
         Cache-Control: {cache}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes()).await;
    if !head_only {
        let _ = stream.write_all(&body).await;
    }
    let _ = stream.flush().await;
}

/// Baca baris permintaan HTTP dan buang headernya.
async fn read_request(stream: &mut TcpStream) -> Option<(String, String)> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        let n = stream.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") || buf.len() >= MAX_HEAD {
            break;
        }
    }

    let head = String::from_utf8_lossy(&buf);
    let mut parts = head.lines().next()?.split_whitespace();
    let method = parts.next()?.to_string();
    let path = parts.next()?.split('?').next()?.to_string();
    Some((method, path))
}

async fn stream_events(
    mut stream: TcpStream,
    hub: Hub,
    alive: Arc<AtomicBool>,
    head_only: bool,
) {
    let header = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\
                  Cache-Control: no-cache\r\nConnection: keep-alive\r\n\
                  X-Accel-Buffering: no\r\n\r\n";
    if stream.write_all(header.as_bytes()).await.is_err() || head_only {
        return;
    }

    let (style, history, live) = hub.snapshot();
    // Gaya dikirim lebih dulu supaya halaman langsung memakai setelan terkini.
    for frame in std::iter::once(style).chain(history) {
        if write_event(&mut stream, &frame).await.is_err() {
            return;
        }
    }
    let _ = stream.flush().await;

    let mut ticker = tokio::time::interval(TICK);
    let mut ticks: u32 = 0;
    loop {
        tokio::select! {
            _ = ticker.tick() => {
                if !alive.load(Ordering::Relaxed) {
                    break;
                }
                ticks += 1;
                if ticks % PING_EVERY_TICKS == 0 {
                    // Komentar SSE: menjaga koneksi tetap hidup dan mendeteksi klien mati.
                    if stream.write_all(b": ping\n\n").await.is_err() {
                        break;
                    }
                    let _ = stream.flush().await;
                }
            }
            received = live.recv() => match received {
                Ok(frame) => {
                    if write_event(&mut stream, &frame).await.is_err() {
                        break;
                    }
                    let _ = stream.flush().await;
                }
                Err(_) => break,
            },
        }
    }
}

async fn write_event(stream: &mut TcpStream, frame: &str) -> std::io::Result<()> {
    stream.write_all(b"data: ").await?;
    stream.write_all(frame.as_bytes()).await?;
    stream.write_all(b"\n\n").await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ChatMsg, Emote, Kind};

    #[test]
    fn avatar_sama_dipakai_ulang() {
        let hub = Hub::new(&Config::default());
        let kunci_a = hub.image(b"\x89PNG\r\n\x1a\nisi");
        let kunci_b = hub.image(b"\x89PNG\r\n\x1a\nisi");
        assert_eq!(kunci_a, kunci_b);
        assert_eq!(hub.lock().images.len(), 1);
    }

    #[test]
    fn pesan_menjadi_json_untuk_obs() {
        let hub = Hub::new(&Config::default());
        let msg = ChatMsg {
            author: "Budi".into(),
            avatar: Some(b"\xff\xd8jpeg".to_vec()),
            is_owner: false,
            is_mod: true,
            is_member: true,
            is_verified: false,
            kind: Kind::SuperChat {
                amount: "Rp 10.000".into(),
                tier: 3,
            },
            text: "halo :catJAM:".into(),
            emotes: vec![Emote {
                name: "catJAM".into(),
                png: Arc::new(b"\x89PNG\r\n\x1a\nx".to_vec()),
            }],
        };
        hub.publish(&Event::Chat(msg));

        let inner = hub.lock();
        assert_eq!(inner.history.len(), 1);
        let frame: serde_json::Value = serde_json::from_str(&inner.history[0]).unwrap();
        assert_eq!(frame["type"], "msg");
        assert_eq!(frame["kind"], "superchat");
        assert_eq!(frame["tier"], 3);
        assert_eq!(frame["amount"], "Rp 10.000");
        assert!(frame["avatar"].as_str().unwrap().starts_with("/img/"));
        assert!(frame["emotes"]["catJAM"].as_str().unwrap().starts_with("/img/"));
    }

    async fn http_get(port: u16, path: &str) -> String {
        let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).await.unwrap();
        let request = format!("GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n");
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut out = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            let n = stream.read(&mut buf).await.unwrap();
            if n == 0 {
                break;
            }
            out.extend_from_slice(&buf[..n]);
        }
        String::from_utf8_lossy(&out).into_owned()
    }

    #[tokio::test]
    async fn server_menyajikan_halaman_css_dan_stream() {
        let hub = Hub::new(&Config::default());
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let alive = Arc::new(AtomicBool::new(true));
        let (stop_tx, stop_rx) = async_channel::unbounded::<()>();
        let server = tokio::spawn(accept_loop(listener, hub.clone(), stop_rx, alive));

        let page = http_get(port, "/").await;
        assert!(page.starts_with("HTTP/1.1 200 OK"), "{page}");
        assert!(page.contains("app.js"));

        let css = http_get(port, "/style.css").await;
        assert!(css.contains(".msg .body"), "{css}");

        // Stream SSE: frame gaya dikirim lebih dulu, lalu pesan baru.
        let mut sse = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).await.unwrap();
        sse.write_all(b"GET /events HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .await
            .unwrap();
        let mut text = String::new();
        let mut buf = [0u8; 4096];
        for _ in 0..32 {
            let read = tokio::time::timeout(Duration::from_millis(500), sse.read(&mut buf)).await;
            match read {
                Ok(Ok(0)) | Ok(Err(_)) | Err(_) => break,
                Ok(Ok(n)) => {
                    text.push_str(&String::from_utf8_lossy(&buf[..n]));
                    if text.contains(r#""type":"style""#) {
                        break;
                    }
                }
            }
        }
        assert!(text.contains("text/event-stream"), "{text}");
        assert!(text.contains(r#""type":"style""#), "{text}");

        // Pesan baru harus sampai ke klien yang sudah terhubung.
        hub.publish(&Event::Chat(ChatMsg {
            author: "Budi".into(),
            avatar: Some(b"\x89PNG\r\n\x1a\navatar".to_vec()),
            is_owner: false,
            is_mod: false,
            is_member: false,
            is_verified: false,
            kind: Kind::Text,
            text: "halo".into(),
            emotes: Vec::new(),
        }));

        let mut text = text;
        for _ in 0..32 {
            if text.contains(r#""type":"msg""#) {
                break;
            }
            let read = tokio::time::timeout(Duration::from_millis(500), sse.read(&mut buf)).await;
            match read {
                Ok(Ok(0)) | Ok(Err(_)) | Err(_) => break,
                Ok(Ok(n)) => text.push_str(&String::from_utf8_lossy(&buf[..n])),
            }
        }
        assert!(text.contains(r#""type":"msg""#), "{text}");
        assert!(text.contains("Budi"), "{text}");

        let key = text
            .split("\"/img/")
            .nth(1)
            .and_then(|rest| rest.split('"').next())
            .expect("URL gambar avatar")
            .to_string();
        let image = http_get(port, &format!("/img/{key}")).await;
        assert!(image.starts_with("HTTP/1.1 200 OK"), "{image}");
        assert!(image.contains("Content-Type: image/png"), "{image}");

        stop_tx.send(()).await.unwrap();
        server.await.unwrap();
    }

    #[tokio::test]
    async fn kontrol_menyalakan_dan_mematikan_server() {
        let port = 18765;
        let hub = Hub::new(&Config::default());
        let ctl = ObsCtl::spawn(hub, port);
        assert!(!ctl.state().running);

        ctl.start();
        let mut running = false;
        for _ in 0..100 {
            if ctl.state().running {
                running = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(running, "server tidak menyala: {:?}", ctl.state());

        let page = http_get(port, "/").await;
        assert!(page.starts_with("HTTP/1.1 200 OK"), "{page}");

        ctl.stop();
        let mut stopped = false;
        for _ in 0..100 {
            if !ctl.state().running {
                stopped = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(stopped, "server tidak berhenti: {:?}", ctl.state());
    }

    #[test]
    fn css_memakai_setelan_config() {
        let mut cfg = Config::default();
        cfg.bg_opacity = 0.5;
        cfg.avatar_px = 24;
        cfg.msg_gap = 9;
        let css = Style::from_config(&cfg).css();
        assert!(css.contains("rgba(0, 0, 0, 0.500)"));
        assert!(css.contains("width: 24px"));
        assert!(css.contains("gap: 9px"));
    }
}

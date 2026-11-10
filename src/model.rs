use std::sync::Arc;

#[derive(Clone, Debug)]
pub enum Kind {
    Text,
    SuperChat { amount: String, tier: u8 },
    SuperSticker { amount: String, tier: u8 },
    Member,
    System,
}

/// Emoji custom YouTube yang sudah diunduh sebagai PNG.
#[derive(Clone, Debug)]
pub struct Emote {
    /// Kunci ternormalisasi, mis. `catJAM` (tanpa `:` dan `_`).
    pub name: String,
    pub png: Arc<Vec<u8>>,
}

#[derive(Clone, Debug)]
pub struct ChatMsg {
    pub author: String,
    pub avatar: Option<Vec<u8>>,
    pub is_owner: bool,
    pub is_mod: bool,
    pub is_member: bool,
    pub is_verified: bool,
    pub kind: Kind,
    /// Teks pesan; shortcode emoji custom masih tersisa di sini dan diganti
    /// gambar oleh UI memakai `emotes`.
    pub text: String,
    /// Emoji custom yang muncul di `text`.
    pub emotes: Vec<Emote>,
}

impl ChatMsg {
    pub fn system(t: impl Into<String>) -> Self {
        ChatMsg {
            author: String::new(),
            avatar: None,
            is_owner: false,
            is_mod: false,
            is_member: false,
            is_verified: false,
            kind: Kind::System,
            text: t.into(),
            emotes: Vec::new(),
        }
    }
}

/// Normalisasi shortcut emoji YouTube (`:_nama_:` / `:nama:`) menjadi kunci
/// yang sama untuk kedua bentuk.
pub fn emote_key(shortcut: &str) -> String {
    let s = shortcut.trim_matches(':');
    let s = s.strip_prefix('_').unwrap_or(s);
    let s = s.strip_suffix('_').unwrap_or(s);
    s.to_string()
}

#[derive(Clone, Copy, Debug)]
pub enum Ctl {
    Toggle,
    Edit,
    Lock,
    Clear,
    Quit,
}

impl Ctl {
    pub fn parse(s: &str) -> Option<Ctl> {
        match s {
            "toggle" => Some(Ctl::Toggle),
            "edit" => Some(Ctl::Edit),
            "lock" => Some(Ctl::Lock),
            "clear" => Some(Ctl::Clear),
            "quit" => Some(Ctl::Quit),
            _ => None,
        }
    }
}

/// Permintaan mulai/sambung ulang live chat dari panel aplikasi.
#[derive(Clone, Debug)]
pub struct JobRequest {
    pub video: String,
    pub api_key: String,
}

pub enum Event {
    Chat(ChatMsg),
    Status(String),
    Ctl(Ctl),
}

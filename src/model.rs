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

/// Angka statistik live yang ditampilkan di layar.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LiveStats {
    /// Penonton yang sedang menonton saat ini (`concurrentViewers`).
    pub viewers: Option<u64>,
    /// Jumlah subscriber kanal.
    pub subscribers: Option<u64>,
    /// Total tayangan video live.
    pub views: Option<u64>,
}

impl LiveStats {
    /// Angka dengan pemisah ribuan gaya Indonesia, mis. `12.345`.
    pub fn format(value: Option<u64>) -> String {
        let Some(v) = value else {
            return "–".to_string();
        };
        let digits = v.to_string();
        let mut out = String::with_capacity(digits.len() + digits.len() / 3);
        for (i, c) in digits.chars().enumerate() {
            if i > 0 && (digits.len() - i) % 3 == 0 {
                out.push('.');
            }
            out.push(c);
        }
        out
    }
}

pub enum Event {
    Chat(ChatMsg),
    Status(String),
    Stats(LiveStats),
    Ctl(Ctl),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn angka_statistik_memakai_pemisah_ribuan() {
        assert_eq!(LiveStats::format(Some(0)), "0");
        assert_eq!(LiveStats::format(Some(999)), "999");
        assert_eq!(LiveStats::format(Some(1234)), "1.234");
        assert_eq!(LiveStats::format(Some(1_234_567)), "1.234.567");
        // Angka yang tidak tersedia (mis. kanal menyembunyikan subscriber).
        assert_eq!(LiveStats::format(None), "–");
    }
}

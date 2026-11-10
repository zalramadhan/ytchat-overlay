use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Config {
    /// YouTube Data API v3 key (bisa juga lewat env `YOUTUBE_API_KEY`)
    pub api_key: String,
    /// Link atau ID video live (bisa juga lewat argumen CLI)
    pub video: String,

    /// Nama konektor monitor, mis. "DP-1" / "HDMI-A-1". Kosong = monitor default.
    pub output: String,
    /// Lapisan layer-shell: "overlay" (paling atas, default) atau "top".
    pub layer: String,

    // Posisi & ukuran kotak chat (piksel, margin dari pojok kiri atas monitor)
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,

    // Tampilan
    pub font_family: String,
    pub font_size: f64,
    pub text_color: String,
    /// 0.0 = sepenuhnya transparan, 1.0 = hitam pekat (latar tiap pesan)
    pub bg_opacity: f64,
    pub show_avatars: bool,
    /// Ukuran avatar dalam piksel (12–48)
    pub avatar_px: i32,
    /// Jarak antar kotak pesan dalam piksel (0–20)
    pub msg_gap: i32,
    pub max_messages: usize,
    /// Pesan mulai memudar setelah N detik (0 = tidak pernah memudar)
    pub fade_after_secs: f64,
    pub fade_duration_secs: f64,

    /// Interval polling minimum (ms). Makin besar = hemat kuota API.
    pub poll_min_ms: u64,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            api_key: String::new(),
            video: String::new(),
            output: String::new(),
            layer: "overlay".into(),
            x: 30,
            y: 200,
            width: 420,
            height: 500,
            font_family: "Noto Sans".into(),
            font_size: 16.0,
            text_color: "#ffffff".into(),
            bg_opacity: 0.25,
            show_avatars: true,
            avatar_px: 16,
            msg_gap: 4,
            max_messages: 40,
            fade_after_secs: 30.0,
            fade_duration_secs: 3.0,
            poll_min_ms: 5000,
        }
    }
}

impl Config {
    pub fn path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("ytchat-overlay")
            .join("config.toml")
    }

    fn read_file() -> Option<Config> {
        let s = std::fs::read_to_string(Self::path()).ok()?;
        match toml::from_str(&s) {
            Ok(c) => Some(c),
            Err(e) => {
                eprintln!("Config tidak valid ({}): {e}", Self::path().display());
                None
            }
        }
    }

    pub fn load() -> Self {
        match Self::read_file() {
            Some(c) => c,
            None => {
                let c = Config::default();
                if !Self::path().exists() {
                    c.save();
                }
                c
            }
        }
    }

    pub fn save(&self) {
        let p = Self::path();
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        if let Ok(s) = toml::to_string_pretty(self) {
            let _ = std::fs::write(p, s);
        }
    }

    /// Simpan setelan tampilan dari panel (opacity, font, avatar, jarak, monitor).
    pub fn save_look(cfg: &Config) {
        let mut c = Self::read_file().unwrap_or_default();
        c.bg_opacity = cfg.bg_opacity;
        c.font_size = cfg.font_size;
        c.avatar_px = cfg.avatar_px;
        c.msg_gap = cfg.msg_gap;
        c.output = cfg.output.clone();
        c.save();
    }

    /// Simpan posisi/ukuran saja, tanpa menulis API key dari env / argumen CLI ke disk.
    pub fn save_geometry(x: i32, y: i32, w: i32, h: i32) {
        let mut c = Self::read_file().unwrap_or_default();
        c.x = x;
        c.y = y;
        c.width = w;
        c.height = h;
        c.save();
    }
}

use crate::{
    config::Config,
    model::{emote_key, ChatMsg, Emote, Event, Kind, LiveStats},
};
use async_channel::Sender;
use serde::Deserialize;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::time::sleep;

const API: &str = "https://www.googleapis.com/youtube/v3";
/// Statistik live tidak perlu secepat chat; 30 detik menghemat kuota API.
const STATS_EVERY: Duration = Duration::from_secs(30);
const UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

// ---------- bentuk JSON ----------
#[derive(Deserialize)]
struct VideoList {
    #[serde(default)]
    items: Vec<VideoItem>,
}
#[derive(Deserialize)]
struct VideoItem {
    #[serde(rename = "liveStreamingDetails")]
    live: Option<Live>,
    snippet: Option<VideoSnippet>,
    statistics: Option<Counts>,
}
#[derive(Deserialize)]
struct Live {
    #[serde(rename = "activeLiveChatId")]
    chat_id: Option<String>,
    /// Jumlah penonton saat ini; hanya ada selama live berlangsung.
    #[serde(rename = "concurrentViewers")]
    viewers: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct VideoSnippet {
    channel_id: Option<String>,
}
/// Angka dari `part=statistics` (dikirim API sebagai string).
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Counts {
    view_count: Option<String>,
    subscriber_count: Option<String>,
}
#[derive(Deserialize)]
struct ChannelList {
    #[serde(default)]
    items: Vec<ChannelItem>,
}
#[derive(Deserialize)]
struct ChannelItem {
    statistics: Option<Counts>,
}

/// Hasil satu panggilan `videos.list`: ruang chat + angka awal statistik.
struct VideoInfo {
    chat_id: String,
    channel_id: Option<String>,
    stats: LiveStats,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChatPage {
    next_page_token: Option<String>,
    polling_interval_millis: Option<u64>,
    #[serde(default)]
    items: Vec<Item>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Item {
    #[serde(default)]
    id: String,
    snippet: Snippet,
    author_details: Option<Author>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Snippet {
    #[serde(rename = "type")]
    kind: String,
    display_message: Option<String>,
    super_chat_details: Option<SuperChat>,
    super_sticker_details: Option<SuperSticker>,
    member_milestone_chat_details: Option<Milestone>,
    new_sponsor_details: Option<NewSponsor>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SuperChat {
    amount_display_string: Option<String>,
    user_comment: Option<String>,
    tier: Option<u8>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SuperSticker {
    amount_display_string: Option<String>,
    tier: Option<u8>,
    super_sticker_metadata: Option<StickerMeta>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StickerMeta {
    alt_text: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Milestone {
    member_month: Option<u64>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NewSponsor {
    member_level_name: Option<String>,
}
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct Author {
    #[serde(default)]
    display_name: String,
    profile_image_url: Option<String>,
    #[serde(default)]
    is_verified: bool,
    #[serde(default)]
    is_chat_owner: bool,
    #[serde(default)]
    is_chat_moderator: bool,
    #[serde(default)]
    is_chat_sponsor: bool,
}

// ---------- emoji ----------
enum EmojiDef {
    /// Emoji standar: cukup diganti karakternya (dirender font).
    Char(String),
    /// Emoji custom kanal: harus diunduh sebagai gambar.
    Image(String),
}

// ---------- util ----------
fn is_id_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-' || c == '_'
}

fn take_id(s: &str) -> Option<String> {
    let id: String = s.chars().take_while(|c| is_id_char(*c)).take(11).collect();
    (id.len() == 11).then_some(id)
}

pub fn video_id(input: &str) -> Option<String> {
    let s = input.trim();
    if s.len() == 11 && s.chars().all(is_id_char) {
        return Some(s.to_string());
    }
    if let Some(i) = s.find("v=") {
        if let Some(id) = take_id(&s[i + 2..]) {
            return Some(id);
        }
    }
    for marker in ["youtu.be/", "/live/", "/shorts/", "/embed/"] {
        if let Some(i) = s.find(marker) {
            if let Some(id) = take_id(&s[i + marker.len()..]) {
                return Some(id);
            }
        }
    }
    None
}

fn api_err(body: &str) -> String {
    let v: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
    let reason = v["error"]["errors"][0]["reason"].as_str().unwrap_or("");
    let msg = v["error"]["message"].as_str().unwrap_or("");
    if reason.is_empty() && msg.is_empty() {
        body.chars().take(120).collect()
    } else {
        format!("{reason}: {msg}")
    }
}

async fn say(tx: &Sender<Event>, s: impl Into<String>) {
    let _ = tx.send(Event::Status(s.into())).await;
}

/// Semua token `:...:` di dalam teks, sebagai (awal, akhir, kunci ternormalisasi).
fn emote_tokens(text: &str) -> Vec<(usize, usize, String)> {
    let mut out = Vec::new();
    let mut pos = 0;
    while let Some(rel) = text[pos..].find(':') {
        let start = pos + rel;
        let Some(rel2) = text[start + 1..].find(':') else {
            break;
        };
        let end = start + 1 + rel2; // indeks ':' penutup
        if end > start + 1 {
            let key = emote_key(&text[start + 1..end]);
            if !key.is_empty() {
                out.push((start, end + 1, key));
                pos = end + 1;
                continue;
            }
        }
        pos = start + 1;
    }
    out
}

/// Ganti shortcode emoji standar dengan karakter aslinya; shortcode emoji custom
/// dibiarkan supaya UI bisa menyisipkan gambar.
fn apply_emoji_chars(text: &str, map: &HashMap<String, EmojiDef>) -> String {
    let tokens = emote_tokens(text);
    if tokens.is_empty() {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for (start, end, key) in tokens {
        out.push_str(&text[last..start]);
        match map.get(&key) {
            Some(EmojiDef::Char(c)) => out.push_str(c),
            _ => out.push_str(&text[start..end]),
        }
        last = end;
    }
    out.push_str(&text[last..]);
    out
}

fn custom_emote_urls(text: &str, map: &HashMap<String, EmojiDef>) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for (_, _, key) in emote_tokens(text) {
        if out.len() >= 24 {
            break;
        }
        if out.iter().any(|(k, _)| *k == key) {
            continue;
        }
        if let Some(EmojiDef::Image(url)) = map.get(&key) {
            out.push((key, url.clone()));
        }
    }
    out
}

/// Ambil JSON `ytInitialData` dari HTML halaman live chat.
fn extract_initial_data(html: &str) -> Option<&str> {
    let idx = html.find("ytInitialData")?;
    let start = idx + html[idx..].find('{')?;
    let bytes = html.as_bytes();
    let mut depth = 0usize;
    let mut in_str = false;
    let mut esc = false;
    for (i, &b) in bytes.iter().enumerate().skip(start) {
        if in_str {
            if esc {
                esc = false;
            } else if b == b'\\' {
                esc = true;
            } else if b == b'"' {
                in_str = false;
            }
            continue;
        }
        match b {
            b'"' => in_str = true,
            b'{' => depth += 1,
            b'}' => {
                if depth == 0 {
                    return None;
                }
                depth -= 1;
                if depth == 0 {
                    return Some(&html[start..=i]);
                }
            }
            _ => {}
        }
    }
    None
}

fn thumb_url(image: &serde_json::Value) -> Option<String> {
    image["thumbnails"]
        .as_array()?
        .iter()
        .rev()
        .find_map(|t| t["url"].as_str())
        .map(str::to_string)
}

/// Telusuri seluruh JSON dan kumpulkan definisi emoji (emojiId + shortcuts + image).
fn collect_emojis(v: &serde_json::Value, out: &mut HashMap<String, EmojiDef>) {
    match v {
        serde_json::Value::Object(o) => {
            let id = o.get("emojiId").and_then(|x| x.as_str());
            let shortcuts = o.get("shortcuts").and_then(|x| x.as_array());
            let image = o.get("image").filter(|x| x.is_object());
            if let (Some(id), Some(shortcuts), Some(image)) = (id, shortcuts, image) {
                let custom = o
                    .get("isCustomEmoji")
                    .and_then(|x| x.as_bool())
                    .unwrap_or_else(|| id.contains('/'));
                let url = thumb_url(image);
                for s in shortcuts.iter().filter_map(|x| x.as_str()) {
                    let key = emote_key(s);
                    if key.is_empty() {
                        continue;
                    }
                    let def = if custom {
                        match &url {
                            Some(u) => EmojiDef::Image(u.clone()),
                            None => continue,
                        }
                    } else if id.chars().count() <= 2 {
                        EmojiDef::Char(id.to_string())
                    } else {
                        continue;
                    };
                    out.entry(key).or_insert(def);
                }
            }
            for val in o.values() {
                collect_emojis(val, out);
            }
        }
        serde_json::Value::Array(a) => {
            for val in a {
                collect_emojis(val, out);
            }
        }
        _ => {}
    }
}

async fn load_emojis(client: &reqwest::Client, vid: &str) -> HashMap<String, EmojiDef> {
    let mut map = HashMap::new();
    let url = format!("https://www.youtube.com/live_chat?is_popout=1&v={vid}&hl=en");
    let req = client
        .get(url)
        .header("accept-language", "en-US,en;q=0.9")
        .header("cookie", "SOCS=CAI");
    let html = match req.send().await {
        Ok(r) => match r.text().await {
            Ok(t) => t,
            Err(_) => return map,
        },
        Err(_) => return map,
    };
    let Some(json) = extract_initial_data(&html) else {
        return map;
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else {
        return map;
    };
    collect_emojis(&v, &mut map);
    map
}

fn parse_count(value: Option<&str>) -> Option<u64> {
    value.and_then(|s| s.trim().parse::<u64>().ok())
}

/// Ambil ruang chat, ID kanal, dan statistik awal dari satu panggilan `videos.list`.
async fn video_info(
    client: &reqwest::Client,
    key: &str,
    vid: &str,
) -> Result<VideoInfo, String> {
    let r = client
        .get(format!("{API}/videos"))
        .query(&[
            ("part", "liveStreamingDetails,snippet,statistics"),
            ("id", vid),
            ("key", key),
        ])
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let st = r.status();
    let body = r.text().await.map_err(|e| e.to_string())?;
    if !st.is_success() {
        return Err(api_err(&body));
    }
    info_from_body(&body)
}

/// Pisahkan pembacaan JSON dari HTTP-nya supaya bisa diuji tanpa jaringan.
fn info_from_body(body: &str) -> Result<VideoInfo, String> {
    let v: VideoList = serde_json::from_str(body).map_err(|e| e.to_string())?;
    let item = v
        .items
        .into_iter()
        .next()
        .ok_or_else(|| "video tidak ditemukan".to_string())?;
    let live = item.live.as_ref();
    let chat_id = live
        .and_then(|l| l.chat_id.clone())
        .ok_or_else(|| "belum live (activeLiveChatId kosong)".to_string())?;
    let viewers = parse_count(live.and_then(|l| l.viewers.as_deref()));
    let views = parse_count(item.statistics.as_ref().and_then(|s| s.view_count.as_deref()));
    Ok(VideoInfo {
        chat_id,
        channel_id: item.snippet.and_then(|s| s.channel_id),
        stats: LiveStats {
            viewers,
            subscribers: None,
            views,
        },
    })
}

/// Jumlah subscriber kanal; `None` kalau kanal menyembunyikannya.
async fn channel_subscribers(
    client: &reqwest::Client,
    key: &str,
    channel: &str,
) -> Option<u64> {
    let r = client
        .get(format!("{API}/channels"))
        .query(&[("part", "statistics"), ("id", channel), ("key", key)])
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .ok()?;
    let body = r.text().await.ok()?;
    let list: ChannelList = serde_json::from_str(&body).ok()?;
    list.items
        .into_iter()
        .next()
        .and_then(|c| c.statistics)
        .and_then(|s| parse_count(s.subscriber_count.as_deref()))
}

/// Statistik terkini: penonton sekarang, total tayangan, dan subscriber.
async fn fetch_stats(
    client: &reqwest::Client,
    key: &str,
    vid: &str,
    channel: Option<&str>,
    previous: LiveStats,
) -> Result<LiveStats, String> {
    let r = client
        .get(format!("{API}/videos"))
        .query(&[
            ("part", "liveStreamingDetails,statistics"),
            ("id", vid),
            ("key", key),
        ])
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let st = r.status();
    let body = r.text().await.map_err(|e| e.to_string())?;
    if !st.is_success() {
        return Err(api_err(&body));
    }
    let mut stats = stats_from_body(&body, previous)?;
    if let Some(channel) = channel {
        if let Some(subs) = channel_subscribers(client, key, channel).await {
            stats.subscribers = Some(subs);
        }
    }
    Ok(stats)
}

fn stats_from_body(body: &str, previous: LiveStats) -> Result<LiveStats, String> {
    let list: VideoList = serde_json::from_str(body).map_err(|e| e.to_string())?;
    let item = list
        .items
        .into_iter()
        .next()
        .ok_or_else(|| "video tidak ditemukan".to_string())?;
    Ok(LiveStats {
        viewers: parse_count(item.live.as_ref().and_then(|l| l.viewers.as_deref())),
        subscribers: previous.subscribers,
        views: parse_count(item.statistics.as_ref().and_then(|s| s.view_count.as_deref())),
    })
}

async fn fetch_bytes(
    client: &reqwest::Client,
    cache: &mut HashMap<String, Option<Vec<u8>>>,
    url: &str,
) -> Option<Vec<u8>> {
    if let Some(v) = cache.get(url) {
        return v.clone();
    }
    let got: Option<Vec<u8>> = async {
        let r = client
            .get(url)
            .timeout(Duration::from_secs(8))
            .send()
            .await
            .ok()?;
        r.bytes().await.ok().map(|b| b.to_vec())
    }
    .await;
    if cache.len() > 500 {
        cache.clear();
    }
    cache.insert(url.to_string(), got.clone());
    got
}

async fn convert(
    it: Item,
    client: &reqwest::Client,
    cache: &mut HashMap<String, Option<Vec<u8>>>,
    emojis: &HashMap<String, EmojiDef>,
    want_avatars: bool,
) -> Option<ChatMsg> {
    let a = it.author_details.unwrap_or_default();
    let sn = it.snippet;
    let kind_str = sn.kind.clone();
    let display = sn.display_message.clone().unwrap_or_default();

    let (kind, text) = match kind_str.as_str() {
        "textMessageEvent" => (Kind::Text, display),
        "superChatEvent" => {
            let d = sn.super_chat_details?;
            (
                Kind::SuperChat {
                    amount: d.amount_display_string.unwrap_or_default(),
                    tier: d.tier.unwrap_or(1),
                },
                d.user_comment.unwrap_or_default(),
            )
        }
        "superStickerEvent" => {
            let d = sn.super_sticker_details?;
            let alt = d
                .super_sticker_metadata
                .and_then(|m| m.alt_text)
                .unwrap_or_default();
            (
                Kind::SuperSticker {
                    amount: d.amount_display_string.unwrap_or_default(),
                    tier: d.tier.unwrap_or(1),
                },
                alt,
            )
        }
        "newSponsorEvent" => {
            let level = sn
                .new_sponsor_details
                .and_then(|d| d.member_level_name)
                .unwrap_or_default();
            let t = if !display.is_empty() {
                display
            } else if level.is_empty() {
                "Member baru bergabung".to_string()
            } else {
                format!("Member baru bergabung ({level})")
            };
            (Kind::Member, t)
        }
        "memberMilestoneChatEvent" => {
            let t = if !display.is_empty() {
                display
            } else {
                let d = sn.member_milestone_chat_details;
                match d.and_then(|d| d.member_month) {
                    Some(m) => format!("Member selama {m} bulan"),
                    None => "Member milestone".to_string(),
                }
            };
            (Kind::Member, t)
        }
        "membershipGiftingEvent" | "giftMembershipReceivedEvent" => (
            Kind::Member,
            if display.is_empty() {
                "Membagikan membership".to_string()
            } else {
                display
            },
        ),
        _ => return None,
    };

    let text = apply_emoji_chars(&text, emojis);
    let mut emotes = Vec::new();
    for (name, url) in custom_emote_urls(&text, emojis) {
        if let Some(png) = fetch_bytes(client, cache, &url).await {
            emotes.push(Emote {
                name,
                png: Arc::new(png),
            });
        }
    }

    let avatar = match &a.profile_image_url {
        Some(u) if want_avatars => fetch_bytes(client, cache, u).await,
        _ => None,
    };

    Some(ChatMsg {
        author: a.display_name,
        avatar,
        is_owner: a.is_chat_owner,
        is_mod: a.is_chat_moderator,
        is_member: a.is_chat_sponsor,
        is_verified: a.is_verified,
        kind,
        text,
        emotes,
    })
}

// ---------- loop utama ----------
pub async fn run(cfg: Config, tx: Sender<Event>) {
    if cfg.api_key.trim().is_empty() {
        say(
            &tx,
            format!(
                "API key kosong. Isi `api_key` di {} atau set env YOUTUBE_API_KEY.",
                Config::path().display()
            ),
        )
        .await;
        return;
    }
    let Some(vid) = video_id(&cfg.video) else {
        say(
            &tx,
            "Link/ID video live belum diisi. Jalankan: ytchat-overlay <link-live> atau isi `video` di config.",
        )
        .await;
        return;
    };

    let client = match reqwest::Client::builder().user_agent(UA).build() {
        Ok(c) => c,
        Err(e) => {
            say(&tx, format!("Gagal membuat HTTP client: {e}")).await;
            return;
        }
    };

    say(&tx, "Menghubungkan ke live chat…").await;
    let info = loop {
        match video_info(&client, &cfg.api_key, &vid).await {
            Ok(info) => break info,
            Err(e) => {
                let wait = if e.contains("quotaExceeded") { 300 } else { 15 };
                say(&tx, format!("Gagal: {e} (coba lagi {wait} dtk)")).await;
                sleep(Duration::from_secs(wait)).await;
            }
        }
    };
    say(&tx, "Tersambung ✔").await;

    let VideoInfo {
        chat_id,
        channel_id,
        stats: initial_stats,
    } = info;

    // Statistik awal: subscribers butuh satu panggilan terpisah ke kanal.
    let mut stats = initial_stats;
    if let Some(channel) = &channel_id {
        if let Some(subs) = channel_subscribers(&client, &cfg.api_key, channel).await {
            stats.subscribers = Some(subs);
        }
    }
    let _ = tx.send(Event::Stats(stats)).await;

    // Daftar emoji diambil sekali dari halaman live chat; gagal = tanpa gambar emoji.
    let emojis = load_emojis(&client, &vid).await;
    if emojis.is_empty() {
        eprintln!("Peta emoji tidak tersedia; emoji custom tampil sebagai teks.");
    }

    let mut cache: HashMap<String, Option<Vec<u8>>> = HashMap::new();
    let mut token: Option<String> = None;
    let mut next_stats = Instant::now() + STATS_EVERY;
    let mut first = true;
    let mut seen: HashSet<String> = HashSet::new();
    let mut seen_order: VecDeque<String> = VecDeque::new();

    loop {
        // Statistik diperbarui lebih jarang daripada chat (hemat kuota).
        if Instant::now() >= next_stats {
            match fetch_stats(
                &client,
                &cfg.api_key,
                &vid,
                channel_id.as_deref(),
                stats,
            )
            .await
            {
                Ok(fresh) if fresh != stats => {
                    stats = fresh;
                    let _ = tx.send(Event::Stats(stats)).await;
                }
                Ok(_) => {}
                Err(e) => eprintln!("Statistik live gagal: {e}"),
            }
            next_stats = Instant::now() + STATS_EVERY;
        }

        let mut q: Vec<(&str, String)> = vec![
            ("liveChatId", chat_id.clone()),
            ("part", "snippet,authorDetails".into()),
            ("key", cfg.api_key.clone()),
            ("maxResults", "200".into()),
        ];
        if let Some(t) = &token {
            q.push(("pageToken", t.clone()));
        }

        let (status, body) = match client
            .get(format!("{API}/liveChat/messages"))
            .query(&q)
            .send()
            .await
        {
            Ok(r) => {
                let s = r.status();
                (s, r.text().await.unwrap_or_default())
            }
            Err(e) => {
                say(&tx, format!("Jaringan bermasalah: {e}")).await;
                sleep(Duration::from_secs(10)).await;
                continue;
            }
        };

        if !status.is_success() {
            let e = api_err(&body);
            if e.contains("liveChatEnded") {
                say(&tx, "Live chat sudah berakhir.").await;
                return;
            }
            let wait = if e.contains("quotaExceeded") { 300 } else { 15 };
            say(&tx, format!("Error API: {e} (coba lagi {wait} dtk)")).await;
            sleep(Duration::from_secs(wait)).await;
            continue;
        }

        let page: ChatPage = match serde_json::from_str(&body) {
            Ok(p) => p,
            Err(e) => {
                say(&tx, format!("Respons tidak terbaca: {e}")).await;
                sleep(Duration::from_secs(10)).await;
                continue;
            }
        };
        token = page.next_page_token;

        // Halaman pertama berisi riwayat chat lama; tampilkan 8 terakhir saja.
        let skip = if first {
            page.items.len().saturating_sub(8)
        } else {
            0
        };
        first = false;
        for it in page.items.into_iter().skip(skip) {
            // Jaga-jaga terhadap halaman yang ter-fetch ulang saat retry.
            if !it.id.is_empty() && !seen.insert(it.id.clone()) {
                continue;
            }
            if !it.id.is_empty() {
                seen_order.push_back(it.id.clone());
                if seen_order.len() > 4000 {
                    if let Some(old) = seen_order.pop_front() {
                        seen.remove(&old);
                    }
                }
            }
            if let Some(m) = convert(it, &client, &mut cache, &emojis, cfg.show_avatars).await {
                let _ = tx.send(Event::Chat(m)).await;
            }
        }

        let wait = page
            .polling_interval_millis
            .unwrap_or(5000)
            .max(cfg.poll_min_ms);
        sleep(Duration::from_millis(wait)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_id_dari_berbagai_bentuk() {
        assert_eq!(video_id("dQw4w9WgXcQ").as_deref(), Some("dQw4w9WgXcQ"));
        assert_eq!(
            video_id("https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=10").as_deref(),
            Some("dQw4w9WgXcQ")
        );
        assert_eq!(
            video_id("https://youtu.be/dQw4w9WgXcQ?si=abc").as_deref(),
            Some("dQw4w9WgXcQ")
        );
        assert_eq!(
            video_id("https://www.youtube.com/live/dQw4w9WgXcQ").as_deref(),
            Some("dQw4w9WgXcQ")
        );
        assert_eq!(video_id("bukan link"), None);
    }

    #[test]
    fn ambil_peta_emoji_dari_html() {
        let html = r#"<script>var ytInitialData = {"contents":{"liveChatRenderer":{"emojis":[
            {"emojiId":"UCabc/def","shortcuts":[":_cat_:",":cat:"],"isCustomEmoji":true,
             "image":{"thumbnails":[{"url":"https://x/24.png","width":24,"height":24},
                                    {"url":"https://x/48.png","width":48,"height":48}]}},
            {"emojiId":"✨","shortcuts":[":sparkles:"],
             "image":{"thumbnails":[{"url":"https://x/e.svg"}]}}
        ]}}};</script>"#;
        let json = extract_initial_data(html).expect("ytInitialData harus ketemu");
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        let mut map = HashMap::new();
        collect_emojis(&v, &mut map);
        match map.get("cat") {
            Some(EmojiDef::Image(u)) => assert_eq!(u, "https://x/48.png"),
            _ => panic!("emoji custom tidak terdeteksi"),
        }
        match map.get("sparkles") {
            Some(EmojiDef::Char(c)) => assert_eq!(c, "✨"),
            _ => panic!("emoji standar tidak terdeteksi"),
        }
    }

    #[test]
    fn statistik_dan_kanal_terbaca_dari_json() {
        let body = r#"{"items":[{
            "snippet":{"channelId":"UCabc"},
            "statistics":{"viewCount":"45678"},
            "liveStreamingDetails":{"activeLiveChatId":"chat1","concurrentViewers":"1234"}
        }]}"#;
        let info = info_from_body(body).expect("info video");
        assert_eq!(info.chat_id, "chat1");
        assert_eq!(info.channel_id.as_deref(), Some("UCabc"));
        assert_eq!(info.stats.viewers, Some(1234));
        assert_eq!(info.stats.views, Some(45678));
        assert_eq!(info.stats.subscribers, None);
    }

    #[test]
    fn subscriber_lama_dipertahankan_saat_angkanya_hilang() {
        let previous = LiveStats {
            viewers: Some(10),
            subscribers: Some(777),
            views: Some(1),
        };
        let body = r#"{"items":[{"statistics":{},"liveStreamingDetails":{}}]}"#;
        let stats = stats_from_body(body, previous).expect("statistik");
        assert_eq!(stats.viewers, None);
        assert_eq!(stats.views, None);
        assert_eq!(stats.subscribers, Some(777));
    }

    #[test]
    fn angka_dari_api_dibaca_aman() {
        assert_eq!(parse_count(None), None);
        assert_eq!(parse_count(Some("")), None);
        assert_eq!(parse_count(Some("abc")), None);
        assert_eq!(parse_count(Some(" 9876 ")), Some(9876));
    }

    #[test]
    fn emoji_standar_diganti_custom_dibiarkan() {
        let mut map = HashMap::new();
        map.insert("sparkles".to_string(), EmojiDef::Char("✨".into()));
        map.insert(
            "cat".to_string(),
            EmojiDef::Image("https://x/48.png".into()),
        );
        let out = apply_emoji_chars("halo :sparkles: :_cat_:", &map);
        assert_eq!(out, "halo ✨ :_cat_:");
        assert_eq!(
            custom_emote_urls(&out, &map),
            vec![("cat".to_string(), "https://x/48.png".to_string())]
        );
    }
}

use crate::{
    config::Config, model::JobRequest, obs::ObsCtl, overlay::Overlay, youtube::video_id,
};
use gtk::prelude::*;
use std::{cell::Cell, rc::Rc, time::Duration};

pub struct Control {
    pub window: gtk::Window,
    hide: gtk::Button,
}

impl Control {
    /// Samakan label tombol tampil/sembunyi dengan keadaan overlay sekarang.
    pub fn sync_state(&self, overlay: &Overlay) {
        sync_hide_label(&self.hide, overlay);
    }
}

/// Panel kecil (jendela biasa, bukan layer-shell) untuk menjalankan overlay
/// tanpa terminal: tempel link, atur posisi, sembunyikan, bersihkan.
pub fn build(
    app: &gtk::Application,
    cfg: &Config,
    overlay: Rc<Overlay>,
    job_tx: async_channel::Sender<JobRequest>,
    obs: ObsCtl,
) -> Rc<Control> {
    let window = gtk::Window::builder()
        .application(app)
        .title("Chat YouTube Live")
        .build();
    window.set_default_size(470, 820);

    // Latar gelap khusus kotak pratinjau, supaya kelihatan seperti di atas game.
    let panel_css = gtk::CssProvider::new();
    panel_css.load_from_data(
        ".preview-stage { background-color: rgba(24, 26, 30, 0.92); \
         border-radius: 8px; padding: 6px; }",
    );
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &panel_css,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }

    let root = gtk::Box::new(gtk::Orientation::Vertical, 10);
    root.set_margin_top(18);
    root.set_margin_bottom(18);
    root.set_margin_start(18);
    root.set_margin_end(18);
    // Panel bisa di-scroll kalau layarnya kecil / jendela dikecilkan.
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scroll.set_max_content_width(470);
    if let Some(display) = gtk::gdk::Display::default() {
        let monitor = display
            .monitors()
            .item(0)
            .and_then(|obj| obj.downcast::<gtk::gdk::Monitor>().ok());
        if let Some(monitor) = monitor {
            let cap = (f64::from(monitor.geometry().height()) * 0.9) as i32;
            scroll.set_max_content_height(cap.max(400));
        }
    }
    scroll.set_child(Some(&root));
    window.set_child(Some(&scroll));

    let title = gtk::Label::new(None);
    title.set_markup("<b><big>Overlay Chat YouTube Live</big></b>");
    title.set_xalign(0.0);
    root.append(&title);

    let intro = gtk::Label::new(Some(
        "Tempel link live, lalu tekan Tampilkan. Overlay muncul di atas game \
         dan tidak menghalangi klik.",
    ));
    intro.set_wrap(true);
    intro.set_xalign(0.0);
    intro.add_css_class("dim-label");
    root.append(&intro);

    // --- Link live ---
    let link_label = gtk::Label::new(Some("Link live YouTube"));
    link_label.set_xalign(0.0);
    root.append(&link_label);

    let link = gtk::Entry::new();
    link.set_placeholder_text(Some("https://www.youtube.com/watch?v=…"));
    link.set_text(&cfg.video);
    link.set_activates_default(false);
    root.append(&link);

    // --- API key ---
    let key_label = gtk::Label::new(Some("API key YouTube Data API v3"));
    key_label.set_xalign(0.0);
    root.append(&key_label);

    let key = gtk::Entry::new();
    key.set_placeholder_text(Some("AIza… (boleh dikosongkan jika sudah lewat env)"));
    key.set_text(&cfg.api_key);
    root.append(&key);

    let start = gtk::Button::with_label("Tampilkan overlay");
    start.add_css_class("suggested-action");
    root.append(&start);

    // --- Tombol kontrol overlay ---
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    actions.set_homogeneous(true);
    let edit = gtk::ToggleButton::with_label("Mode edit posisi");
    edit.set_tooltip_text(Some(
        "Aktifkan, geser overlay dengan mouse, lalu matikan lagi untuk menyimpan posisi.",
    ));
    let hide = gtk::Button::with_label("Sembunyikan");
    let clear = gtk::Button::with_label("Bersihkan chat");
    actions.append(&edit);
    actions.append(&hide);
    actions.append(&clear);
    root.append(&actions);

    // --- Tampilan overlay ---
    root.append(&gtk::Separator::new(gtk::Orientation::Horizontal));

    let look_title = gtk::Label::new(None);
    look_title.set_markup("<b>Tampilan</b>");
    look_title.set_xalign(0.0);
    root.append(&look_title);

    let opacity_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let opacity_label = gtk::Label::new(Some("Opacity latar"));
    opacity_label.set_xalign(0.0);
    opacity_label.set_width_chars(14);
    let opacity = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 100.0, 1.0);
    opacity.set_value((cfg.bg_opacity * 100.0).clamp(0.0, 100.0));
    opacity.set_draw_value(false);
    opacity.set_hexpand(true);
    opacity.set_tooltip_text(Some("Kegelapan kotak di belakang tiap pesan chat."));
    let opacity_value = gtk::Label::new(Some(&format!("{:.0}%", cfg.bg_opacity * 100.0)));
    opacity_value.set_width_chars(4);
    opacity_value.set_xalign(1.0);
    opacity_row.append(&opacity_label);
    opacity_row.append(&opacity);
    opacity_row.append(&opacity_value);
    root.append(&opacity_row);

    let font_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let font_label = gtk::Label::new(Some("Ukuran font"));
    font_label.set_xalign(0.0);
    font_label.set_width_chars(14);
    let font = gtk::SpinButton::with_range(10.0, 48.0, 1.0);
    font.set_value(cfg.font_size.clamp(10.0, 48.0));
    font.set_digits(0);
    font.set_hexpand(true);
    font.set_halign(gtk::Align::End);
    font.set_tooltip_text(Some("Ukuran teks chat pada overlay (piksel)."));
    font_row.append(&font_label);
    font_row.append(&font);
    root.append(&font_row);

    let avatar_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let avatar_label = gtk::Label::new(Some("Ukuran avatar"));
    avatar_label.set_xalign(0.0);
    avatar_label.set_width_chars(14);
    let avatar = gtk::Scale::with_range(gtk::Orientation::Horizontal, 12.0, 48.0, 1.0);
    let avatar_start = cfg.avatar_px.clamp(12, 48);
    avatar.set_value(avatar_start as f64);
    avatar.set_draw_value(false);
    avatar.set_hexpand(true);
    avatar.set_tooltip_text(Some(
        "Ukuran foto profil di samping tiap pesan chat, dalam piksel.",
    ));
    let avatar_value = gtk::Label::new(Some(&format!("{avatar_start} px")));
    avatar_value.set_width_chars(4);
    avatar_value.set_xalign(1.0);
    avatar_row.append(&avatar_label);
    avatar_row.append(&avatar);
    avatar_row.append(&avatar_value);
    root.append(&avatar_row);

    let gap_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let gap_label = gtk::Label::new(Some("Jarak antar pesan"));
    gap_label.set_xalign(0.0);
    gap_label.set_width_chars(14);
    let gap = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 20.0, 1.0);
    let gap_start = cfg.msg_gap.clamp(0, 20);
    gap.set_value(f64::from(gap_start));
    gap.set_draw_value(false);
    gap.set_hexpand(true);
    gap.set_tooltip_text(Some("Spasi vertikal antar kotak pesan chat (piksel)."));
    let gap_value = gtk::Label::new(Some(&format!("{gap_start} px")));
    gap_value.set_width_chars(4);
    gap_value.set_xalign(1.0);
    gap_row.append(&gap_label);
    gap_row.append(&gap);
    gap_row.append(&gap_value);
    root.append(&gap_row);

    let monitor_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let monitor_label = gtk::Label::new(Some("Monitor"));
    monitor_label.set_xalign(0.0);
    monitor_label.set_width_chars(14);
    let connectors = monitor_connectors();
    let mut monitor_names = vec!["Ikuti layar aktif".to_string()];
    monitor_names.extend(connectors.iter().cloned());
    let monitor_refs: Vec<&str> = monitor_names.iter().map(String::as_str).collect();
    let monitor = gtk::DropDown::from_strings(&monitor_refs);
    monitor.set_hexpand(true);
    monitor.set_halign(gtk::Align::End);
    let selected = connectors
        .iter()
        .position(|c| c == cfg.output.trim())
        .map(|i| i as u32 + 1)
        .unwrap_or(0);
    monitor.set_selected(selected);
    monitor.set_tooltip_text(Some("Layar tempat overlay ditampilkan."));
    monitor_row.append(&monitor_label);
    monitor_row.append(&monitor);
    root.append(&monitor_row);

    // --- Pratinjau ---
    root.append(&gtk::Separator::new(gtk::Orientation::Horizontal));

    let preview_title = gtk::Label::new(None);
    preview_title.set_markup("<b>Pratinjau</b>");
    preview_title.set_xalign(0.0);
    root.append(&preview_title);

    let preview = gtk::Box::new(gtk::Orientation::Vertical, 0);
    preview.set_hexpand(true);
    preview.add_css_class("preview-stage");
    fill_preview(&preview, avatar_start);
    root.append(&preview);

    // --- OBS Studio ---
    root.append(&gtk::Separator::new(gtk::Orientation::Horizontal));

    let obs_title = gtk::Label::new(None);
    obs_title.set_markup("<b>OBS Studio</b>");
    obs_title.set_xalign(0.0);
    root.append(&obs_title);

    let obs_desc = gtk::Label::new(Some(
        "Nyalakan server di bawah, lalu di OBS tambahkan source Browser dan tempel \
         URL-nya. Latar transparan dan gayanya sama dengan overlay.",
    ));
    obs_desc.set_wrap(true);
    obs_desc.set_xalign(0.0);
    obs_desc.add_css_class("dim-label");
    root.append(&obs_desc);

    let obs_toggle = gtk::ToggleButton::with_label("Server OBS: mati");
    root.append(&obs_toggle);

    let obs_url_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let obs_url = gtk::Entry::new();
    obs_url.set_text(&obs.url());
    obs_url.set_editable(false);
    obs_url.set_hexpand(true);
    let obs_copy = gtk::Button::with_label("Salin");
    obs_copy.set_sensitive(false);
    obs_url_row.append(&obs_url);
    obs_url_row.append(&obs_copy);
    root.append(&obs_url_row);

    let obs_status = gtk::Label::new(Some(
        "Server mati. Aktifkan untuk memakai chat di OBS.",
    ));
    obs_status.set_wrap(true);
    obs_status.set_xalign(0.0);
    obs_status.add_css_class("dim-label");
    root.append(&obs_status);

    // Tombol mengikuti keadaan server sebenarnya (mis. gagal karena port dipakai).
    let obs_syncing = Rc::new(Cell::new(false));
    {
        let obs_ctl = obs.clone();
        let obs_syncing = obs_syncing.clone();
        obs_toggle.connect_toggled(move |btn| {
            if obs_syncing.get() {
                return;
            }
            if btn.is_active() {
                obs_ctl.start();
            } else {
                obs_ctl.stop();
            }
        });
    }
    {
        let obs_url = obs_url.clone();
        obs_copy.connect_clicked(move |btn| {
            if let Some(display) = gtk::gdk::Display::default() {
                display.clipboard().set_text(&obs_url.text());
                btn.set_label("Tersalin");
            }
        });
    }
    {
        let obs_ctl = obs.clone();
        let obs_toggle = obs_toggle.clone();
        let obs_status = obs_status.clone();
        let obs_url = obs_url.clone();
        let obs_copy = obs_copy.clone();
        let obs_syncing = obs_syncing.clone();
        gtk::glib::timeout_add_local(Duration::from_millis(300), move || {
            let state = obs_ctl.state();
            if obs_url.text() != obs_ctl.url() {
                obs_url.set_text(&obs_ctl.url());
            }
            obs_syncing.set(true);
            obs_toggle.set_active(state.running);
            obs_toggle.set_label(if state.running {
                "Server OBS: aktif"
            } else {
                "Server OBS: mati"
            });
            obs_syncing.set(false);
            obs_copy.set_sensitive(state.running);
            match (state.running, &state.error) {
                (true, _) => obs_status
                    .set_text("Aktif. Di OBS: Sources → + → Browser → tempel URL di atas."),
                (false, Some(err)) => obs_status.set_text(&format!("Gagal: {err}")),
                (false, None) => {
                    obs_status.set_text("Server mati. Aktifkan untuk memakai chat di OBS.")
                }
            }
            gtk::glib::ControlFlow::Continue
        });
    }

    // Nilai terakhir yang benar-benar diterapkan, supaya event kecil dari GTK saat
    // tata letak tidak menimpa setelan tersimpan.
    let last_opacity = Rc::new(Cell::new(cfg.bg_opacity));
    {
        let overlay = overlay.clone();
        let opacity_value = opacity_value.clone();
        let last_opacity = last_opacity.clone();
        opacity.connect_value_changed(move |scale| {
            let percent = scale.value();
            opacity_value.set_text(&format!("{percent:.0}%"));
            let value = percent / 100.0;
            if (value - last_opacity.get()).abs() < 0.004 {
                return;
            }
            last_opacity.set(value);
            overlay.set_opacity(value);
        });
    }

    let last_font = Rc::new(Cell::new(cfg.font_size));
    {
        let overlay = overlay.clone();
        let last_font = last_font.clone();
        font.connect_value_changed(move |spin| {
            let value = spin.value();
            if (value - last_font.get()).abs() < 0.5 {
                return;
            }
            last_font.set(value);
            overlay.set_font_size(value);
        });
    }

    let last_avatar = Rc::new(Cell::new(avatar_start));
    {
        let overlay = overlay.clone();
        let avatar_value = avatar_value.clone();
        let last_avatar = last_avatar.clone();
        let preview = preview.clone();
        avatar.connect_value_changed(move |scale| {
            let px = scale.value().round() as i32;
            avatar_value.set_text(&format!("{px} px"));
            if px == last_avatar.get() {
                return;
            }
            last_avatar.set(px);
            overlay.set_avatar_size(px);
            fill_preview(&preview, px);
        });
    }

    let last_gap = Rc::new(Cell::new(gap_start));
    {
        let overlay = overlay.clone();
        let gap_value = gap_value.clone();
        let last_gap = last_gap.clone();
        gap.connect_value_changed(move |scale| {
            let px = scale.value().round() as i32;
            gap_value.set_text(&format!("{px} px"));
            if px == last_gap.get() {
                return;
            }
            last_gap.set(px);
            overlay.set_gap(px);
        });
    }

    {
        let overlay = overlay.clone();
        monitor.connect_selected_notify(move |dropdown| {
            let index = dropdown.selected();
            let connector = if index == 0 {
                String::new()
            } else {
                connectors
                    .get(index as usize - 1)
                    .cloned()
                    .unwrap_or_default()
            };
            overlay.set_monitor(&connector);
        });
    }

    // --- Status ---
    let status = gtk::Label::new(Some("Siap. Belum ada live yang disambungkan."));
    status.set_wrap(true);
    status.set_xalign(0.0);
    status.set_valign(gtk::Align::Start);
    status.set_vexpand(true);
    status.add_css_class("dim-label");
    root.append(&status);

    let hint = gtk::Label::new(None);
    hint.set_markup(&format!(
        "<small>Setelan tersimpan otomatis di <tt>{}</tt></small>",
        gtk::glib::markup_escape_text(&Config::path().display().to_string())
    ));
    hint.set_xalign(0.0);
    hint.set_wrap(true);
    hint.add_css_class("dim-label");
    root.append(&hint);

    let quit = gtk::Button::with_label("Keluar");
    quit.set_halign(gtk::Align::End);
    root.append(&quit);

    // Status dari background (koneksi, error API, live berakhir) tampil di panel.
    {
        let status = status.clone();
        overlay.on_status(move |s| status.set_text(s));
    }

    // Sinkronkan tombol edit bila mode edit diubah lewat `ctl edit`.
    let syncing = Rc::new(Cell::new(false));
    {
        let edit = edit.clone();
        let syncing = syncing.clone();
        overlay.on_editing_changed(move |on| {
            syncing.set(true);
            edit.set_active(on);
            syncing.set(false);
        });
    }
    {
        let overlay = overlay.clone();
        let syncing = syncing.clone();
        edit.connect_toggled(move |btn| {
            if syncing.get() {
                return;
            }
            if btn.is_active() && !overlay.is_visible() {
                overlay.show();
            }
            overlay.set_editing(btn.is_active());
        });
    }

    {
        let overlay = overlay.clone();
        let hide_btn = hide.clone();
        hide.connect_clicked(move |_| {
            if overlay.is_visible() {
                overlay.hide();
            } else {
                overlay.show();
            }
            sync_hide_label(&hide_btn, &overlay);
        });
    }

    {
        let overlay = overlay.clone();
        clear.connect_clicked(move |_| overlay.clear());
    }

    // Mulai / ganti live.
    {
        let overlay = overlay.clone();
        let link = link.clone();
        let key = key.clone();
        let status = status.clone();
        let edit = edit.clone();
        let hide = hide.clone();
        start.connect_clicked(move |_| {
            let Some(video) = video_id(link.text().trim()) else {
                status.set_text("Link live tidak dikenali. Contoh: https://www.youtube.com/watch?v=XXXXXXXXXXX");
                return;
            };
            let api_key = key.text().trim().to_string();
            if api_key.is_empty() {
                status.set_text(
                    "API key masih kosong. Isi kolom di atas atau jalankan dengan env YOUTUBE_API_KEY.",
                );
                return;
            }

            // Simpan link (dan key hanya jika bukan berasal dari env) ke config.
            let mut saved = Config::load();
            saved.video = video.clone();
            let from_env = std::env::var("YOUTUBE_API_KEY").unwrap_or_default();
            if api_key != from_env {
                saved.api_key = api_key.clone();
            }
            saved.save();

            if let Err(e) = job_tx.try_send(JobRequest {
                video: video.clone(),
                api_key,
            }) {
                status.set_text(&format!("Gagal memulai: {e}"));
                return;
            }

            overlay.clear();
            overlay.show();
            if edit.is_active() {
                edit.set_active(false);
            }
            sync_hide_label(&hide, &overlay);
            status.set_text("Menyambung ke live chat…");
        });
    }

    {
        let window = window.clone();
        quit.connect_clicked(move |_| {
            if let Some(app) = window.application() {
                app.quit();
            } else {
                window.close();
            }
        });
    }

    // Menutup panel = menghentikan overlay sekaligus, supaya tidak ada jendela
    // "hantu" yang tak bisa dikendalikan lagi.
    {
        let window = window.clone();
        let window_for_close = window.clone();
        window.connect_close_request(move |_| {
            if let Some(app) = window_for_close.application() {
                app.quit();
            }
            gtk::glib::Propagation::Proceed
        });
    }

    let control = Rc::new(Control {
        window,
        hide: hide.clone(),
    });

    // Overlay awalnya tersembunyi (atau sudah tampil jika diluncurkan dengan link).
    sync_hide_label(&hide, &overlay);
    control
}

/// Nama konektor monitor yang sedang terpasang, mis. `DP-1`, `HDMI-A-1`.
/// Isi ulang kotak pratinjau; dipanggil saat ukuran avatar berubah.
fn fill_preview(container: &gtk::Box, avatar_px: i32) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }

    container.append(&preview_row(
        avatar_px,
        "Budi",
        "Halo semua, semangat mainnya! 🔥",
        None,
        &["msg"],
        (0.29, 0.52, 0.86),
    ));
    container.append(&preview_row(
        avatar_px,
        "Sinta",
        "GG banget tadi 👏",
        None,
        &["msg", "member"],
        (0.33, 0.70, 0.42),
    ));
    container.append(&preview_row(
        avatar_px,
        "Andi",
        "Semangat terus, jangan menyerah!",
        Some("Rp 10.000"),
        &["msg", "sc3"],
        (0.85, 0.55, 0.20),
    ));
}

/// Satu baris contoh chat dengan gaya yang sama seperti overlay (kelas CSS `.msg`).
fn preview_row(
    avatar_px: i32,
    name: &str,
    text: &str,
    amount: Option<&str>,
    classes: &[&str],
    color: (f64, f64, f64),
) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 5);
    for class in classes {
        row.add_css_class(class);
    }
    row.set_can_target(false);

    let initial = name.chars().next().unwrap_or('?').to_string();
    row.append(&preview_avatar(avatar_px, color, &initial));

    let col = gtk::Box::new(gtk::Orientation::Vertical, 1);
    col.set_hexpand(true);

    let head = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    let name_label = gtk::Label::new(None);
    name_label.set_markup(&format!(
        "<b>{}</b>",
        gtk::glib::markup_escape_text(name)
    ));
    name_label.set_xalign(0.0);
    name_label.add_css_class("author");
    head.append(&name_label);

    if let Some(amount) = amount {
        let amount_label = gtk::Label::new(None);
        amount_label.set_markup(&format!(
            "<b>{}</b>",
            gtk::glib::markup_escape_text(amount)
        ));
        amount_label.set_valign(gtk::Align::Center);
        amount_label.add_css_class("amount");
        head.append(&amount_label);
    }
    col.append(&head);

    let body = gtk::Label::new(None);
    body.set_markup(&gtk::glib::markup_escape_text(text));
    body.set_wrap(true);
    body.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    body.set_xalign(0.0);
    body.add_css_class("body");
    col.append(&body);

    row.append(&col);
    row
}

/// Avatar contoh: lingkaran berwarna dengan inisial, seukuran avatar asli.
fn preview_avatar(px: i32, color: (f64, f64, f64), initial: &str) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::new();
    area.set_content_width(px);
    area.set_content_height(px);
    area.set_valign(gtk::Align::Start);
    area.set_can_target(false);

    let letter = initial.to_string();
    area.set_draw_func(move |_, cr, w, h| {
        let (w, h) = (w as f64, h as f64);
        let radius = w.min(h) / 2.0;
        cr.save().ok();
        cr.arc(w / 2.0, h / 2.0, radius, 0.0, std::f64::consts::TAU);
        cr.set_source_rgb(color.0, color.1, color.2);
        cr.fill().ok();

        cr.set_source_rgba(1.0, 1.0, 1.0, 0.92);
        cr.select_font_face(
            "Sans",
            gtk::cairo::FontSlant::Normal,
            gtk::cairo::FontWeight::Bold,
        );
        cr.set_font_size(radius * 0.95);
        if let Ok(ext) = cr.text_extents(&letter) {
            cr.move_to(
                w / 2.0 - (ext.width() / 2.0 + ext.x_bearing()),
                h / 2.0 - (ext.height() / 2.0 + ext.y_bearing()),
            );
            let _ = cr.show_text(&letter);
        }
        cr.restore().ok();
    });
    area
}

fn monitor_connectors() -> Vec<String> {
    let mut out = Vec::new();
    let Some(display) = gtk::gdk::Display::default() else {
        return out;
    };
    let monitors = display.monitors();
    for i in 0..monitors.n_items() {
        let Some(obj) = monitors.item(i) else {
            continue;
        };
        let Ok(mon) = obj.downcast::<gtk::gdk::Monitor>() else {
            continue;
        };
        let name = match mon.connector() {
            Some(c) if !c.is_empty() => c.to_string(),
            _ => format!("Monitor {}", i + 1),
        };
        if !out.contains(&name) {
            out.push(name);
        }
    }
    out
}

fn sync_hide_label(hide: &gtk::Button, overlay: &Overlay) {
    hide.set_label(if overlay.is_visible() {
        "Sembunyikan"
    } else {
        "Tampilkan"
    });
}

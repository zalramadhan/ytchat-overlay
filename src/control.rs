use crate::{config::Config, model::JobRequest, overlay::Overlay, youtube::video_id};
use gtk::prelude::*;
use std::{cell::Cell, rc::Rc};

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
) -> Rc<Control> {
    let window = gtk::Window::builder()
        .application(app)
        .title("Chat YouTube Live")
        .default_width(470)
        .resizable(false)
        .build();

    let root = gtk::Box::new(gtk::Orientation::Vertical, 10);
    root.set_margin_top(18);
    root.set_margin_bottom(18);
    root.set_margin_start(18);
    root.set_margin_end(18);
    window.set_child(Some(&root));

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

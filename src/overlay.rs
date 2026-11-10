use crate::{
    config::Config,
    model::{emote_key, ChatMsg, Ctl, Event, JobRequest, Kind},
};
use gtk::prelude::*;
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    rc::Rc,
    time::Duration,
};

const APP_ID: &str = "id.ytchat.Overlay";
const FADE_STEP_MS: u64 = 50;

pub fn run(
    cfg: Config,
    rx: async_channel::Receiver<Event>,
    job_tx: async_channel::Sender<JobRequest>,
    start_video: Option<String>,
) {
    let app = gtk::Application::builder().application_id(APP_ID).build();
    let cfg_for_activate = cfg.clone();
    let existing: Rc<RefCell<Option<Rc<crate::control::Control>>>> = Rc::new(RefCell::new(None));
    app.connect_activate(move |app| {
        // Aplikasi sudah berjalan (mis. ikon diklik dua kali): cukup tampilkan panelnya.
        if let Some(panel) = existing.borrow().as_ref() {
            panel.window.present();
            return;
        }
        // Harus setelah GTK diinisialisasi oleh Application.
        if !gtk4_layer_shell::is_supported() {
            eprintln!(
                "Layer shell tidak didukung oleh compositor ini. Overlay ini butuh Wayland + KDE Plasma (atau wlroots)."
            );
            app.quit();
            return;
        }
        let overlay = Rc::new(build(app, &cfg_for_activate, rx.clone()));
        let panel = crate::control::build(app, &cfg_for_activate, overlay.clone(), job_tx.clone());
        *existing.borrow_mut() = Some(panel.clone());

        match &start_video {
            // Dipanggil dengan link: langsung tampilkan overlay, panel tetap tersedia.
            Some(video) => {
                let _ = job_tx.try_send(JobRequest {
                    video: video.clone(),
                    api_key: cfg_for_activate.api_key.clone(),
                });
                overlay.show();
                panel.window.present();
                panel.sync_state(&overlay);
            }
            None => {
                panel.window.present();
            }
        }

    });

    // Jangan teruskan argumen CLI ke GTK (argumen sudah kita proses sendiri).
    app.run_with_args::<&str>(&[]);
}

/// Pegangan ke jendela overlay, dipakai panel aplikasi.
#[derive(Clone)]
pub struct Overlay {
    ui: Rc<Ui>,
    persist_gen: Rc<Cell<u64>>,
}

impl Overlay {
    pub fn show(&self) {
        self.ui.window.borrow().present();
    }

    pub fn hide(&self) {
        self.ui.window.borrow().set_visible(false);
    }

    pub fn is_visible(&self) -> bool {
        self.ui.window.borrow().is_visible()
    }

    pub fn set_editing(&self, on: bool) {
        self.ui.set_editing(on);
    }

    pub fn clear(&self) {
        self.ui.clear();
    }

    /// Opacity latar pesan (0.0–1.0); langsung terlihat pada semua pesan.
    pub fn set_opacity(&self, opacity: f64) {
        self.ui.set_look(Some(opacity), None);
        self.schedule_persist();
    }

    /// Ukuran font chat dalam piksel.
    pub fn set_font_size(&self, size: f64) {
        self.ui.set_look(None, Some(size));
        self.schedule_persist();
    }

    /// Pindahkan overlay ke monitor lain (nama konektor; kosong = layar aktif).
    /// Output layer-shell hanya bisa dipilih saat surface dibuat, jadi jendelanya
    /// dibuat ulang dengan isi chat yang sama.
    pub fn set_monitor(&self, connector: &str) {
        let connector = connector.trim().to_string();
        let ui = self.ui.clone();
        if ui.cfg.borrow().output == connector {
            return;
        }

        let old = ui.window.borrow().clone();
        let display = gtk::prelude::WidgetExt::display(&old);
        let monitor = if connector.is_empty() {
            None
        } else {
            match find_monitor(&display, &connector) {
                Some(mon) => Some(mon),
                None => {
                    ui.report_status(&format!(
                        "Monitor `{connector}` tidak ditemukan; overlay tetap di layar aktif."
                    ));
                    return;
                }
            }
        };

        {
            let mut cfg = ui.cfg.borrow_mut();
            cfg.output = connector;
        }

        let was_visible = old.is_visible();
        let left = old.margin(Edge::Left);
        let top = old.margin(Edge::Top);
        let cfg = ui.cfg.borrow().clone();

        let new_window = create_window(&ui.app, &cfg, monitor.as_ref());
        new_window.set_margin(Edge::Left, left);
        new_window.set_margin(Edge::Top, top);

        // Pindahkan isi chat ke jendela baru tanpa kehilangan pesan.
        old.set_child(None::<&gtk::Widget>);
        new_window.set_child(Some(&ui.root));
        *ui.window.borrow_mut() = new_window.clone();

        let ui_after_map = ui.clone();
        new_window.connect_map(move |_| apply_input_region(&ui_after_map));

        if was_visible {
            new_window.present();
        }
        old.close();
        ui.persist_look();
    }

    pub fn on_status<F: Fn(&str) + 'static>(&self, f: F) {
        *self.ui.status_sink.borrow_mut() = Some(Rc::new(f));
    }

    pub fn on_editing_changed<F: Fn(bool) + 'static>(&self, f: F) {
        *self.ui.editing_sink.borrow_mut() = Some(Rc::new(f));
    }

    /// Simpan setelan tampilan sekali saja setelah perubahan terakhir (mis. sambil
    /// menggeser slider opacity).
    fn schedule_persist(&self) {
        let generation = self.persist_gen.get() + 1;
        self.persist_gen.set(generation);
        let ui = self.ui.clone();
        let counter = self.persist_gen.clone();
        gtk::glib::timeout_add_local_once(Duration::from_millis(400), move || {
            if counter.get() == generation {
                ui.persist_look();
            }
        });
    }
}

fn create_window(
    app: &gtk::Application,
    cfg: &Config,
    monitor: Option<&gtk::gdk::Monitor>,
) -> gtk::Window {
    let window = gtk::Window::builder()
        .application(app)
        .title("ytchat-overlay")
        .decorated(false)
        .resizable(false)
        .default_width(cfg.width.max(80))
        .default_height(cfg.height.max(60))
        .build();

    // Kelas khusus supaya CSS transparan tidak ikut kena jendela panel.
    window.add_css_class("chat-window");
    window.init_layer_shell();
    window.set_namespace(Some("ytchat-overlay"));
    window.set_layer(layer_of(cfg));
    window.set_anchor(Edge::Top, true);
    window.set_anchor(Edge::Left, true);
    window.set_margin(Edge::Left, cfg.x);
    window.set_margin(Edge::Top, cfg.y);
    window.set_exclusive_zone(0);
    window.set_keyboard_mode(KeyboardMode::None);
    if let Some(mon) = monitor {
        window.set_monitor(Some(mon));
    }
    window
}

fn build(app: &gtk::Application, cfg: &Config, rx: async_channel::Receiver<Event>) -> Overlay {
    let display = gtk::gdk::Display::default();
    let monitor = cfg.output.trim();
    let monitor = match &display {
        Some(d) if !monitor.is_empty() => match find_monitor(d, monitor) {
            Some(mon) => Some(mon),
            None => {
                eprintln!("Monitor `{}` tidak ditemukan; pakai monitor default.", cfg.output);
                None
            }
        },
        _ => None,
    };

    let window = create_window(app, cfg, monitor.as_ref());

    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.add_css_class("chat-root");
    root.set_halign(gtk::Align::Fill);
    root.set_valign(gtk::Align::End);
    root.set_vexpand(true);
    window.set_child(Some(&root));

    // Satu provider CSS dipakai ulang supaya opacity/ukuran font bisa diubah live.
    let provider = gtk::CssProvider::new();
    if let Some(display) = &display {
        gtk::style_context_add_provider_for_display(
            display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
    install_css(&provider, cfg);

    let ui = Rc::new(Ui {
        app: app.clone(),
        cfg: RefCell::new(cfg.clone()),
        window: RefCell::new(window.clone()),
        provider,
        root: root.clone(),
        rows: Rc::new(RefCell::new(VecDeque::new())),
        next_id: Cell::new(0),
        editing: Cell::new(false),
        drag_from: Cell::new((cfg.x, cfg.y)),
        status_sink: RefCell::new(None),
        editing_sink: RefCell::new(None),
    });

    // Geser posisi saat mode edit (input region aktif hanya di mode itu).
    let drag = gtk::GestureDrag::new();
    {
        let ui = ui.clone();
        drag.connect_drag_begin(move |_, _, _| {
            let window = ui.window.borrow();
            let left = window.margin(Edge::Left);
            let top = window.margin(Edge::Top);
            ui.drag_from.set((left, top));
        });
    }
    {
        let ui = ui.clone();
        drag.connect_drag_update(move |_, dx, dy| {
            if !ui.editing.get() {
                return;
            }
            let (left, top) = ui.drag_from.get();
            let window = ui.window.borrow();
            window.set_margin(Edge::Left, left + dx as i32);
            window.set_margin(Edge::Top, top + dy as i32);
        });
    }
    root.add_controller(drag);

    // Klik-tembus: input region kosong setelah surface benar-benar ada.
    let ui_after_map = ui.clone();
    window.connect_map(move |_| apply_input_region(&ui_after_map));

    // Aliran event dari thread YouTube/IPC ke UI thread.
    let ui_events = ui.clone();
    gtk::glib::timeout_add_local(Duration::from_millis(80), move || {
        while let Ok(ev) = rx.try_recv() {
            match ev {
                Event::Chat(m) => ui_events.push_chat(m),
                Event::Status(s) => {
                    ui_events.report_status(&s);
                    ui_events.push_chat(ChatMsg::system(s));
                }
                Event::Ctl(c) => ui_events.handle_ctl(c),
            }
        }
        gtk::glib::ControlFlow::Continue
    });

    Overlay {
        ui,
        persist_gen: Rc::new(Cell::new(0)),
    }
}

fn layer_of(cfg: &Config) -> Layer {
    match cfg.layer.trim().to_ascii_lowercase().as_str() {
        "top" => Layer::Top,
        _ => Layer::Overlay,
    }
}

fn find_monitor(display: &gtk::gdk::Display, name: &str) -> Option<gtk::gdk::Monitor> {
    let monitors = display.monitors();
    for i in 0..monitors.n_items() {
        let obj = monitors.item(i)?;
        let mon = obj.downcast::<gtk::gdk::Monitor>().ok()?;
        if mon.connector().map(|c| c.as_str() == name).unwrap_or(false) {
            return Some(mon);
        }
    }
    None
}

fn css_color(value: &str) -> String {
    let t = value.trim();
    let hex = t.len() >= 4
        && t.len() <= 9
        && t.starts_with('#')
        && t[1..].chars().all(|c| c.is_ascii_hexdigit());
    if hex {
        t.to_string()
    } else {
        "#ffffff".to_string()
    }
}

fn install_css(provider: &gtk::CssProvider, cfg: &Config) {
    let bg = cfg.bg_opacity.clamp(0.0, 1.0);
    let fg = css_color(&cfg.text_color);
    let font = {
        let f = cfg.font_family.replace(['"', '\\'], "");
        if f.trim().is_empty() {
            "sans-serif".to_string()
        } else {
            f
        }
    };
    let px = cfg.font_size.clamp(8.0, 72.0);
    let size_pt = px * 0.75;
    let author_pt = (px * 0.8).max(7.0) * 0.75;
    let radius = (px * 0.5).round() as i32;

    let css = format!(
        r#"
        .chat-window {{ background-color: transparent; }}
        .chat-root {{ background-color: transparent; padding: 4px; }}
        .msg {{
            background-color: rgba(0, 0, 0, {bg:.3});
            border-radius: {radius}px;
            padding: 3px 8px;
            margin: 2px 0px;
            color: {fg};
            font-family: "{font}";
        }}
        .msg .body, .msg .body text {{
            background-color: transparent;
            color: {fg};
            font-family: "{font}";
            font-size: {size_pt:.2}pt;
        }}
        .msg .author {{
            font-family: "{font}";
            font-size: {author_pt:.2}pt;
            font-weight: 700;
            color: #c7d2fe;
        }}
        .msg .amount {{
            font-family: "{font}";
            font-size: {author_pt:.2}pt;
            font-weight: 700;
            color: #ffffff;
        }}
        .msg .badge {{
            font-family: "{font}";
            font-size: {author_pt:.2}pt;
            font-weight: 700;
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
        .msg.sc1 .body, .msg.sc1 .body text, .msg.sc2 .body, .msg.sc2 .body text,
        .msg.sc4 .body, .msg.sc4 .body text, .msg.sc5 .body, .msg.sc5 .body text,
        .msg.sc6 .body, .msg.sc6 .body text, .msg.sc7 .body, .msg.sc7 .body text {{
            color: #ffffff;
        }}
        .msg.sc3 .body, .msg.sc3 .body text {{ color: #1b1b1b; }}
        .msg.sc1 .author, .msg.sc2 .author, .msg.sc4 .author,
        .msg.sc5 .author, .msg.sc6 .author, .msg.sc7 .author {{ color: #ffffff; }}
        .msg.sc3 .author {{ color: #1b1b1b; }}
        .chat-root.editing {{
            background-color: rgba(20, 20, 20, 0.35);
            border: 1px dashed rgba(255, 255, 255, 0.7);
        }}
        picture.avatar {{ border-radius: 4px; }}
        "#
    );

    provider.load_from_data(&css);
}

fn apply_input_region(ui: &Ui) {
    let window = ui.window.borrow().clone();
    let Some(surface) = window.surface() else {
        return;
    };
    if ui.editing.get() {
        let cfg = ui.cfg.borrow();
        let rx = cfg.width.max(80);
        let ry = cfg.height.max(60);
        let region = gtk::cairo::Region::create_rectangle(&gtk::cairo::RectangleInt::new(
            0, 0, rx, ry,
        ));
        surface.set_input_region(&region);
    } else {
        surface.set_input_region(&gtk::cairo::Region::create());
    }
}

struct Ui {
    app: gtk::Application,
    cfg: RefCell<Config>,
    window: RefCell<gtk::Window>,
    provider: gtk::CssProvider,
    root: gtk::Box,
    rows: Rc<RefCell<VecDeque<(u64, gtk::Box)>>>,
    next_id: Cell<u64>,
    editing: Cell<bool>,
    drag_from: Cell<(i32, i32)>,
    status_sink: RefCell<Option<Rc<dyn Fn(&str)>>>,
    editing_sink: RefCell<Option<Rc<dyn Fn(bool)>>>,
}

impl Ui {
    fn report_status(&self, text: &str) {
        if let Some(f) = self.status_sink.borrow().as_ref() {
            f(text);
        }
    }

    /// Terapkan opacity/ukuran font baru dan muat ulang CSS (tanpa restart).
    fn set_look(&self, opacity: Option<f64>, font_size: Option<f64>) {
        {
            let mut cfg = self.cfg.borrow_mut();
            if let Some(v) = opacity {
                cfg.bg_opacity = v.clamp(0.0, 1.0);
            }
            if let Some(v) = font_size {
                cfg.font_size = v.clamp(8.0, 72.0);
            }
        }
        install_css(&self.provider, &self.cfg.borrow());
    }

    /// Tulis setelan tampilan ke config, tanpa menyentuh API key dari env.
    fn persist_look(&self) {
        let cfg = self.cfg.borrow();
        Config::save_look(cfg.bg_opacity, cfg.font_size, &cfg.output);
    }

    fn handle_ctl(&self, ctl: Ctl) {
        match ctl {
            Ctl::Toggle => {
                let window = self.window.borrow();
                window.set_visible(!window.is_visible());
            }
            Ctl::Edit => self.set_editing(!self.editing.get()),
            Ctl::Lock => self.set_editing(false),
            Ctl::Clear => self.clear(),
            Ctl::Quit => {
                if self.editing.get() {
                    self.set_editing(false);
                }
                self.app.quit();
            }
        }
    }

    fn set_editing(&self, on: bool) {
        if self.editing.get() == on {
            return;
        }
        self.editing.set(on);
        if on {
            self.root.add_css_class("editing");
        } else {
            self.root.remove_css_class("editing");
            let (left, top) = {
                let window = self.window.borrow();
                (window.margin(Edge::Left), window.margin(Edge::Top))
            };
            let (width, height) = {
                let cfg = self.cfg.borrow();
                (cfg.width, cfg.height)
            };
            Config::save_geometry(left, top, width, height);
        }
        apply_input_region(self);
        if let Some(f) = self.editing_sink.borrow().as_ref() {
            f(on);
        }
    }

    fn clear(&self) {
        for (_, row) in self.rows.borrow_mut().drain(..) {
            if row.parent().is_some() {
                self.root.remove(&row);
            }
        }
    }

    fn push_chat(&self, msg: ChatMsg) {
        let row = self.build_row(&msg);
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        self.root.append(&row);
        self.rows.borrow_mut().push_back((id, row.clone()));

        let (max, fade_after, fade_duration) = {
            let cfg = self.cfg.borrow();
            (
                cfg.max_messages.max(1),
                cfg.fade_after_secs,
                cfg.fade_duration_secs.max(0.1),
            )
        };
        while self.rows.borrow().len() > max {
            let popped = self.rows.borrow_mut().pop_front();
            match popped {
                Some((_, old)) => {
                    if old.parent().is_some() {
                        self.root.remove(&old);
                    }
                }
                None => break,
            }
        }

        if fade_after > 0.0 {
            schedule_fade(
                self.root.clone(),
                row,
                self.rows.clone(),
                id,
                fade_after,
                fade_duration,
            );
        }
    }

    fn build_row(&self, msg: &ChatMsg) -> gtk::Box {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        row.add_css_class("msg");
        row.set_can_target(false);

        match &msg.kind {
            Kind::SuperChat { tier, .. } | Kind::SuperSticker { tier, .. } => {
                row.add_css_class(&format!("sc{}", (*tier).clamp(1, 7)))
            }
            Kind::Member => row.add_css_class("member"),
            Kind::System => row.add_css_class("system"),
            Kind::Text => {}
        }
        if msg.is_owner {
            row.add_css_class("owner");
        }
        if msg.is_mod {
            row.add_css_class("mod");
        }
        if msg.is_member && !matches!(msg.kind, Kind::Member) {
            row.add_css_class("member");
        }

        let (show_avatars, avatar_size) = {
            let cfg = self.cfg.borrow();
            (cfg.show_avatars, cfg.avatar_size)
        };
        if show_avatars {
            if let Some(bytes) = &msg.avatar {
                if let Ok(texture) = gtk::gdk::Texture::from_bytes(&gtk::glib::Bytes::from(&bytes[..]))
                {
                    let pic = gtk::Picture::for_paintable(&texture);
                    pic.set_content_fit(gtk::ContentFit::Cover);
                    pic.set_can_shrink(true);
                    pic.set_size_request(avatar_size.max(12), avatar_size.max(12));
                    pic.set_valign(gtk::Align::Start);
                    pic.set_can_target(false);
                    pic.add_css_class("avatar");
                    row.append(&pic);
                }
            }
        }

        let col = gtk::Box::new(gtk::Orientation::Vertical, 1);
        col.set_hexpand(true);
        col.set_can_target(false);

        if !msg.author.is_empty() {
            let head = gtk::Box::new(gtk::Orientation::Horizontal, 4);
            head.set_can_target(false);

            let name = gtk::Label::new(None);
            name.set_markup(&author_markup(msg));
            name.set_xalign(0.0);
            name.set_can_target(false);
            name.add_css_class("author");
            head.append(&name);

            if let Kind::SuperChat { amount, .. } | Kind::SuperSticker { amount, .. } = &msg.kind {
                if !amount.is_empty() {
                    let amt = gtk::Label::new(None);
                    amt.set_markup(&format!(
                        "<b>{}</b>",
                        gtk::glib::markup_escape_text(amount)
                    ));
                    amt.set_can_target(false);
                    amt.set_valign(gtk::Align::Center);
                    amt.add_css_class("amount");
                    head.append(&amt);
                }
            }

            col.append(&head);
        }

        if matches!(msg.kind, Kind::System) {
            let label = gtk::Label::new(None);
            label.set_markup(&gtk::glib::markup_escape_text(&msg.text));
            label.set_wrap(true);
            label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
            label.set_xalign(0.0);
            label.set_can_target(false);
            label.add_css_class("body");
            col.append(&label);
        } else {
            col.append(&self.body_view(msg));
        }

        row.append(&col);
        row
    }

    fn body_view(&self, msg: &ChatMsg) -> gtk::TextView {
        let buf = gtk::TextBuffer::new(None::<&gtk::TextTagTable>);
        let view = gtk::TextView::with_buffer(&buf);
        view.set_editable(false);
        view.set_cursor_visible(false);
        view.set_can_focus(false);
        view.set_wrap_mode(gtk::WrapMode::WordChar);
        view.set_left_margin(0);
        view.set_right_margin(0);
        view.set_top_margin(0);
        view.set_bottom_margin(0);
        view.set_pixels_above_lines(0);
        view.set_pixels_below_lines(0);
        view.set_valign(gtk::Align::Start);
        view.set_vexpand(false);
        view.set_can_target(false);
        view.add_css_class("body");

        let font_size = self.cfg.borrow().font_size;
        let emotes: std::collections::HashMap<&str, &[u8]> = msg
            .emotes
            .iter()
            .map(|e| (e.name.as_str(), e.png.as_slice()))
            .collect();

        let mut iter = buf.end_iter();
        let text = msg.text.as_str();
        let mut pos = 0usize;
        while pos < text.len() {
            let Some(rel) = text[pos..].find(':') else {
                break;
            };
            let start = pos + rel;
            let Some(rel2) = text[start + 1..].find(':') else {
                break;
            };
            let end = start + 1 + rel2;
            let key = emote_key(&text[start + 1..end]);
            if let Some(png) = emotes.get(key.as_str()) {
                if let Ok(texture) =
                    gtk::gdk::Texture::from_bytes(&gtk::glib::Bytes::from(*png))
                {
                    if start > pos {
                        buf.insert(&mut iter, &text[pos..start]);
                    }
                    let img = gtk::Image::from_paintable(Some(&texture));
                    img.set_pixel_size((font_size * 1.25).round() as i32);
                    img.set_can_target(false);
                    img.add_css_class("emoji");
                    let anchor = buf.create_child_anchor(&mut iter);
                    view.add_child_at_anchor(&img, &anchor);
                    pos = end + 1;
                    continue;
                }
            }
            buf.insert(&mut iter, &text[pos..=start]);
            pos = start + 1;
        }
        if pos < text.len() {
            buf.insert(&mut iter, &text[pos..]);
        }

        view
    }
}

fn author_markup(msg: &ChatMsg) -> String {
    let mut out = String::new();
    if msg.is_owner {
        out.push_str("<span foreground=\"#ffd54f\">OWNER</span> ");
    } else if msg.is_mod {
        out.push_str("<span foreground=\"#8ab4f8\">MOD</span> ");
    } else if msg.is_member {
        out.push_str("<span foreground=\"#7ee08a\">MEMBER</span> ");
    }
    out.push_str(&gtk::glib::markup_escape_text(&msg.author));
    if msg.is_verified {
        out.push_str(" <span foreground=\"#9aa7b8\">✔</span>");
    }
    out
}

fn schedule_fade(
    root: gtk::Box,
    row: gtk::Box,
    rows: Rc<RefCell<VecDeque<(u64, gtk::Box)>>>,
    id: u64,
    after_secs: f64,
    duration_secs: f64,
) {
    let steps = ((duration_secs * 1000.0) / FADE_STEP_MS as f64).ceil().max(1.0) as u32;
    gtk::glib::timeout_add_local_once(
        Duration::from_secs_f64(after_secs.max(0.0)),
        move || {
            if row.parent().is_none() {
                return;
            }
            let mut step = 0u32;
            gtk::glib::timeout_add_local(Duration::from_millis(FADE_STEP_MS), move || {
                if row.parent().is_none() {
                    return gtk::glib::ControlFlow::Break;
                }
                step += 1;
                let opacity = 1.0 - (step as f64 / steps as f64);
                row.set_opacity(opacity.max(0.0));
                if step >= steps {
                    row.set_opacity(0.0);
                    root.remove(&row);
                    rows.borrow_mut().retain(|(rid, _)| *rid != id);
                    return gtk::glib::ControlFlow::Break;
                }
                gtk::glib::ControlFlow::Continue
            });
        },
    );
}

use crate::{config::Config, model::LiveStats};
use gtk::prelude::*;
use gtk4_layer_shell::{Edge, KeyboardMode, LayerShell};
use std::{cell::{Cell, RefCell}, rc::Rc};

/// Kartu kecil berisi jumlah penonton, subscriber, dan total tayangan live.
/// Jendelanya layer-shell seperti overlay chat: menempel di atas game dan
/// klik-tembus, kecuali saat mode edit.
#[derive(Clone)]
pub struct Stats {
    ui: Rc<StatsUi>,
}

impl Stats {
    /// Perbarui angka yang tampil; nilai yang tidak tersedia tampil sebagai `–`.
    pub fn set(&self, stats: LiveStats) {
        self.ui
            .viewers
            .set_text(&LiveStats::format(stats.viewers));
        self.ui
            .subscribers
            .set_text(&LiveStats::format(stats.subscribers));
        self.ui.views.set_text(&LiveStats::format(stats.views));
    }

    pub fn set_enabled(&self, on: bool) {
        {
            let mut cfg = self.ui.cfg.borrow_mut();
            if cfg.stats == on {
                return;
            }
            cfg.stats = on;
        }
        if on {
            self.ui.window.borrow().present();
        } else {
            self.ui.window.borrow().set_visible(false);
        }
        self.ui.persist();
    }

    pub fn set_editing(&self, on: bool) {
        if self.ui.editing.get() == on {
            return;
        }
        self.ui.editing.set(on);
        if on {
            self.ui.root.add_css_class("editing");
        } else {
            self.ui.root.remove_css_class("editing");
            self.ui.persist();
        }
        self.ui.apply_input_region();
    }

    pub fn set_font_size(&self, px: f64) {
        let px = px.clamp(10.0, 40.0);
        {
            let mut cfg = self.ui.cfg.borrow_mut();
            if (cfg.stats_font_size - px).abs() < 0.01 {
                return;
            }
            cfg.stats_font_size = px;
        }
        self.ui.install_css();
        self.ui.persist();
    }

    /// Pindahkan kartu ke monitor lain (nama konektor; kosong = layar aktif).
    /// Output layer-shell hanya bisa dipilih saat surface dibuat, jadi jendelanya
    /// dibuat ulang lalu isinya dipindahkan.
    pub fn set_monitor(&self, connector: &str) {
        let connector = connector.trim().to_string();
        if self.ui.cfg.borrow().output == connector {
            return;
        }
        let old = self.ui.window.borrow().clone();
        let display = WidgetExt::display(&old);
        let monitor = if connector.is_empty() {
            None
        } else {
            match crate::overlay::find_monitor(&display, &connector) {
                Some(mon) => Some(mon),
                None => return,
            }
        };
        let Some(app) = old.application() else {
            return;
        };

        {
            let mut cfg = self.ui.cfg.borrow_mut();
            cfg.output = connector;
        }

        let was_visible = old.is_visible();
        let left = old.margin(Edge::Left);
        let top = old.margin(Edge::Top);
        let cfg = self.ui.cfg.borrow().clone();
        let new_window = create_window(&app, &cfg, monitor.as_ref());
        new_window.set_margin(Edge::Left, left);
        new_window.set_margin(Edge::Top, top);

        old.set_child(None::<&gtk::Widget>);
        new_window.set_child(Some(&self.ui.root));
        *self.ui.window.borrow_mut() = new_window.clone();

        let ui_after_map = self.ui.clone();
        new_window.connect_map(move |_| ui_after_map.apply_input_region());

        if was_visible {
            new_window.present();
        }
        old.close();
    }
}

pub fn build(app: &gtk::Application, cfg: &Config) -> Stats {
    let display = gtk::gdk::Display::default();
    let output = cfg.output.trim();
    let monitor = match &display {
        Some(d) if !output.is_empty() => crate::overlay::find_monitor(d, output),
        _ => None,
    };

    let window = create_window(app, cfg, monitor.as_ref());

    let root = gtk::Box::new(gtk::Orientation::Vertical, 2);
    root.add_css_class("stats-box");
    root.set_halign(gtk::Align::Start);
    window.set_child(Some(&root));

    let provider = gtk::CssProvider::new();
    if let Some(display) = &display {
        gtk::style_context_add_provider_for_display(
            display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }

    let mut values = Vec::new();
    for label in ["Penonton", "Subscriber", "Total view"] {
        root.append(&stat_row(label, &mut values));
    }
    let viewers = values[0].clone();
    let subscribers = values[1].clone();
    let views = values[2].clone();

    let ui = Rc::new(StatsUi {
        cfg: RefCell::new(cfg.clone()),
        window: RefCell::new(window.clone()),
        root: root.clone(),
        provider,
        viewers,
        subscribers,
        views,
        editing: Cell::new(false),
        drag_from: Cell::new((cfg.stats_x, cfg.stats_y)),
    });
    ui.install_css();

    let drag = gtk::GestureDrag::new();
    {
        let ui = ui.clone();
        drag.connect_drag_begin(move |_, _, _| {
            let window = ui.window.borrow();
            ui.drag_from
                .set((window.margin(Edge::Left), window.margin(Edge::Top)));
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

    let ui_after_map = ui.clone();
    window.connect_map(move |_| ui_after_map.apply_input_region());

    let stats = Stats { ui: ui.clone() };
    if cfg.stats {
        window.present();
    }
    stats
}

fn stat_row(label: &str, values: &mut Vec<gtk::Label>) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let name = gtk::Label::new(Some(label));
    name.add_css_class("stats-label");
    name.set_xalign(0.0);
    name.set_width_chars(10);
    name.set_hexpand(true);

    let value = gtk::Label::new(Some("–"));
    value.add_css_class("stats-value");
    value.set_xalign(1.0);
    values.push(value.clone());

    row.append(&name);
    row.append(&value);
    row
}

fn create_window(
    app: &gtk::Application,
    cfg: &Config,
    monitor: Option<&gtk::gdk::Monitor>,
) -> gtk::Window {
    let window = gtk::Window::builder()
        .application(app)
        .title("ytchat-stats")
        .decorated(false)
        .resizable(false)
        .build();

    window.add_css_class("stats-window");
    window.init_layer_shell();
    window.set_namespace(Some("ytchat-stats"));
    window.set_layer(crate::overlay::layer_of(cfg));
    window.set_anchor(Edge::Top, true);
    window.set_anchor(Edge::Left, true);
    window.set_margin(Edge::Left, cfg.stats_x);
    window.set_margin(Edge::Top, cfg.stats_y);
    window.set_exclusive_zone(0);
    window.set_keyboard_mode(KeyboardMode::None);
    if let Some(mon) = monitor {
        window.set_monitor(Some(mon));
    }
    window
}

struct StatsUi {
    cfg: RefCell<Config>,
    window: RefCell<gtk::Window>,
    root: gtk::Box,
    provider: gtk::CssProvider,
    viewers: gtk::Label,
    subscribers: gtk::Label,
    views: gtk::Label,
    editing: Cell<bool>,
    drag_from: Cell<(i32, i32)>,
}

impl StatsUi {
    fn install_css(&self) {
        let cfg = self.cfg.borrow();
        let bg = cfg.bg_opacity.clamp(0.0, 1.0);
        let px = cfg.stats_font_size.clamp(10.0, 40.0);
        let value_pt = px * 0.75;
        let label_pt = (px * 0.72).max(7.0) * 0.75;
        let radius = (px * 0.6).round() as i32;
        let css = format!(
            r#"
            .stats-window {{ background-color: transparent; }}
            .stats-box {{
                background-color: rgba(0, 0, 0, {bg:.3});
                border-radius: {radius}px;
                padding: 5px 10px;
                border: 2px solid transparent;
            }}
            .stats-box.editing {{ border-color: rgba(90, 169, 255, 0.95); }}
            .stats-label {{
                color: rgba(255, 255, 255, 0.65);
                font-size: {label_pt:.1}pt;
            }}
            .stats-value {{
                color: #ffffff;
                font-weight: 700;
                font-size: {value_pt:.1}pt;
            }}
            "#
        );
        self.provider.load_from_data(&css);
    }

    fn persist(&self) {
        {
            let window = self.window.borrow();
            let mut cfg = self.cfg.borrow_mut();
            cfg.stats_x = window.margin(Edge::Left);
            cfg.stats_y = window.margin(Edge::Top);
        }
        Config::save_stats(&self.cfg.borrow());
    }

    fn apply_input_region(&self) {
        let window = self.window.borrow().clone();
        let Some(surface) = window.surface() else {
            return;
        };
        if self.editing.get() {
            let (w, h) = (window.width().max(1), window.height().max(1));
            let region =
                gtk::cairo::Region::create_rectangle(&gtk::cairo::RectangleInt::new(0, 0, w, h));
            surface.set_input_region(&region);
        } else {
            surface.set_input_region(&gtk::cairo::Region::create());
        }
    }

}

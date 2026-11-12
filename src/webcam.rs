use crate::config::Config;
use gtk::prelude::*;
use gtk4_layer_shell::{Edge, KeyboardMode, LayerShell};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::{
        atomic::{AtomicBool, AtomicI32, Ordering},
        Arc,
    },
    time::Duration,
};

/// Resolusi yang diminta ke kamera. Preview di layar tingginya mengikuti rasio ini.
const CAPTURE_W: u32 = 640;
const CAPTURE_H: u32 = 480;
const FRAME_INTERVAL_MS: u64 = 33;

/// Satu frame kamera dalam BGRA (urutan yang dipakai cairo `ARgb32` di little-endian).
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

/// Jendela webcam di atas game: transparan di luar gambarnya, klik-tembus,
/// dan posisinya digeser bersama mode edit overlay.
#[derive(Clone)]
pub struct Webcam {
    ui: Rc<CamUi>,
}

impl Webcam {
    pub fn show(&self) {
        self.ui.window.borrow().present();
    }

    pub fn hide(&self) {
        self.ui.window.borrow().set_visible(false);
    }

    /// Nyalakan/matikan webcam: mulai ambil frame, atau hentikan kameranya.
    pub fn set_enabled(&self, on: bool) {
        {
            let mut cfg = self.ui.cfg.borrow_mut();
            if cfg.cam == on {
                return;
            }
            cfg.cam = on;
        }
        if on {
            self.show();
            self.ui.start_capture();
        } else {
            self.ui.stop_capture();
            self.hide();
        }
        self.ui.persist();
    }

    /// Ganti perangkat kamera (mis. `/dev/video2`).
    pub fn set_device(&self, path: &str) {
        let path = path.trim().to_string();
        if path.is_empty() || self.ui.cfg.borrow().cam_device == path {
            return;
        }
        self.ui.cfg.borrow_mut().cam_device = path;
        self.ui.restart_capture();
        self.ui.persist();
    }

    /// Lebar preview di layar; tingginya mengikuti rasio gambar kamera.
    pub fn set_width(&self, px: i32) {
        let px = px.clamp(120, 960);
        {
            let mut cfg = self.ui.cfg.borrow_mut();
            if cfg.cam_width == px {
                return;
            }
            cfg.cam_width = px;
        }
        self.ui.apply_size();
        self.schedule_persist();
    }

    /// Rasio gambar: `"4:3"`, `"16:9"`, atau `"1:1"`. Gambar dipotong (cover)
    /// mengikuti rasio ini, bukan diregangkan.
    pub fn set_ratio(&self, ratio: &str) {
        let ratio = ratio.trim().to_string();
        if ratio.is_empty() || self.ui.cfg.borrow().cam_ratio == ratio {
            return;
        }
        self.ui.cfg.borrow_mut().cam_ratio = ratio;
        self.ui.apply_size();
        self.schedule_persist();
    }

    /// Kecerahan gambar (-100..100); diterapkan ke frame berikutnya.
    pub fn set_brightness(&self, value: i32) {
        let value = value.clamp(-100, 100);
        {
            let mut cfg = self.ui.cfg.borrow_mut();
            if cfg.cam_brightness == value {
                return;
            }
            cfg.cam_brightness = value;
        }
        self.ui.adjust.brightness.store(value, Ordering::Relaxed);
        self.schedule_persist();
    }

    /// Kontras gambar (-100..100); `-100` = abu-abu rata, `100` = dua kali kontras.
    pub fn set_contrast(&self, value: i32) {
        let value = value.clamp(-100, 100);
        {
            let mut cfg = self.ui.cfg.borrow_mut();
            if cfg.cam_contrast == value {
                return;
            }
            cfg.cam_contrast = value;
        }
        self.ui.adjust.contrast.store(value, Ordering::Relaxed);
        self.schedule_persist();
    }

    pub fn set_mirror(&self, on: bool) {
        {
            let mut cfg = self.ui.cfg.borrow_mut();
            if cfg.cam_mirror == on {
                return;
            }
            cfg.cam_mirror = on;
        }
        self.ui.area.queue_draw();
        self.schedule_persist();
    }

    pub fn set_editing(&self, on: bool) {
        if self.ui.editing.get() == on {
            return;
        }
        self.ui.editing.set(on);
        self.ui.area.queue_draw();
        self.ui.apply_input_region();
        if !on {
            self.ui.persist();
        }
    }

    /// Pindahkan webcam ke monitor lain (nama konektor; kosong = layar aktif).
    /// Output layer-shell hanya bisa dipilih saat surface dibuat, jadi jendelanya
    /// dibuat ulang dengan gambar yang sedang berjalan.
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

        {
            let mut cfg = self.ui.cfg.borrow_mut();
            cfg.output = connector;
        }

        let was_visible = old.is_visible();
        let left = old.margin(Edge::Left);
        let top = old.margin(Edge::Top);

        let cfg = self.ui.cfg.borrow().clone();
        let new_window = create_window(&self.ui.app, &cfg, monitor.as_ref());
        new_window.set_margin(Edge::Left, left);
        new_window.set_margin(Edge::Top, top);

        old.set_child(None::<&gtk::Widget>);
        new_window.set_child(Some(&self.ui.area));
        *self.ui.window.borrow_mut() = new_window.clone();

        let ui_after_map = self.ui.clone();
        new_window.connect_map(move |_| ui_after_map.apply_input_region());

        if was_visible {
            new_window.present();
        }
        old.close();
    }

    pub fn on_status<F: Fn(&str) + 'static>(&self, f: F) {
        *self.ui.status_sink.borrow_mut() = Some(Rc::new(f));
    }

    /// Simpan setelan webcam sekali saja setelah perubahan terakhir (mis. sambil
    /// menggeser slider ukuran).
    fn schedule_persist(&self) {
        let generation = self.ui.persist_gen.get() + 1;
        self.ui.persist_gen.set(generation);
        let ui = self.ui.clone();
        gtk::glib::timeout_add_local_once(Duration::from_millis(400), move || {
            if ui.persist_gen.get() == generation {
                ui.persist();
            }
        });
    }
}

pub fn build(app: &gtk::Application, cfg: &Config) -> Webcam {
    let (frames_tx, frames_rx) = async_channel::unbounded::<Frame>();
    let (status_tx, status_rx) = async_channel::unbounded::<String>();

    let display = gtk::gdk::Display::default();
    let monitor = cfg.output.trim();
    let monitor = match &display {
        Some(d) if !monitor.is_empty() => crate::overlay::find_monitor(d, monitor),
        _ => None,
    };

    let window = create_window(app, cfg, monitor.as_ref());

    let area = gtk::DrawingArea::new();
    area.set_hexpand(true);
    area.set_vexpand(true);
    window.set_child(Some(&area));

    let ui = Rc::new(CamUi {
        app: app.clone(),
        cfg: RefCell::new(cfg.clone()),
        window: RefCell::new(window.clone()),
        area: area.clone(),
        surface: RefCell::new(None),
        editing: Cell::new(false),
        drag_from: Cell::new((cfg.cam_x, cfg.cam_y)),
        status_sink: RefCell::new(None),
        capture: RefCell::new(None),
        adjust: Arc::new(Adjust::new(cfg.cam_brightness, cfg.cam_contrast)),
        frames_tx,
        status_tx,
        persist_gen: Cell::new(0),
    });
    ui.apply_size();

    {
        let ui = ui.clone();
        area.set_draw_func(move |_, cr, w, h| draw(&ui, cr, w, h));
    }

    // Geser posisi hanya saat mode edit.
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
    area.add_controller(drag);

    let ui_after_map = ui.clone();
    window.connect_map(move |_| ui_after_map.apply_input_region());

    // Frame & status dari thread kamera.
    {
        let ui = ui.clone();
        gtk::glib::timeout_add_local(Duration::from_millis(FRAME_INTERVAL_MS), move || {
            // Ambil frame terbaru saja; frame yang tertinggal dibuang.
            let mut latest = None;
            while let Ok(frame) = frames_rx.try_recv() {
                latest = Some(frame);
            }
            if let Some(frame) = latest {
                ui.put_frame(frame);
            }
            while let Ok(text) = status_rx.try_recv() {
                ui.report_status(&text);
            }
            gtk::glib::ControlFlow::Continue
        });
    }

    let cam = Webcam { ui: ui.clone() };
    if cfg.cam {
        cam.show();
        ui.start_capture();
    }
    cam
}

fn create_window(
    app: &gtk::Application,
    cfg: &Config,
    monitor: Option<&gtk::gdk::Monitor>,
) -> gtk::Window {
    let window = gtk::Window::builder()
        .application(app)
        .title("ytstream-webcam")
        .decorated(false)
        .resizable(false)
        .build();

    window.add_css_class("cam-window");
    window.init_layer_shell();
    window.set_namespace(Some("ytstream-webcam"));
    window.set_layer(crate::overlay::layer_of(cfg));
    window.set_anchor(Edge::Top, true);
    window.set_anchor(Edge::Left, true);
    window.set_margin(Edge::Left, cfg.cam_x);
    window.set_margin(Edge::Top, cfg.cam_y);
    window.set_exclusive_zone(0);
    window.set_keyboard_mode(KeyboardMode::None);
    if let Some(mon) = monitor {
        window.set_monitor(Some(mon));
    }
    window
}

struct CamUi {
    app: gtk::Application,
    cfg: RefCell<Config>,
    window: RefCell<gtk::Window>,
    area: gtk::DrawingArea,
    surface: RefCell<Option<gtk::cairo::ImageSurface>>,
    editing: Cell<bool>,
    drag_from: Cell<(i32, i32)>,
    status_sink: RefCell<Option<Rc<dyn Fn(&str)>>>,
    capture: RefCell<Option<Capture>>,
    /// Kecerahan/kontras dibagi dengan thread kamera.
    adjust: Arc<Adjust>,
    frames_tx: async_channel::Sender<Frame>,
    status_tx: async_channel::Sender<String>,
    persist_gen: Cell<u64>,
}

impl CamUi {
    fn report_status(&self, text: &str) {
        if let Some(f) = self.status_sink.borrow().as_ref() {
            f(text);
        }
    }

    /// Sesuaikan ukuran jendela dengan lebar setelan dan rasio gambar pilihan.
    fn apply_size(&self) {
        let width = self.cfg.borrow().cam_width.clamp(120, 960);
        let ratio = ratio_of(&self.cfg.borrow().cam_ratio);
        let height = (f64::from(width) / ratio).round() as i32;
        let height = height.clamp(60, 2160);
        self.area.set_content_width(width);
        self.area.set_content_height(height);
        self.window.borrow().set_default_size(width, height);
    }

    fn put_frame(&self, frame: Frame) {
        let (w, h) = (frame.width, frame.height);
        let stride = (w * 4) as i32;
        match gtk::cairo::ImageSurface::create_for_data(
            frame.bgra,
            gtk::cairo::Format::ARgb32,
            w as i32,
            h as i32,
            stride,
        ) {
            Ok(surface) => *self.surface.borrow_mut() = Some(surface),
            Err(e) => {
                self.report_status(&format!("Kamera: gambar tidak bisa dipakai ({e})."));
                return;
            }
        }
        self.area.queue_draw();
    }

    /// Tulis setelan webcam ke config, termasuk posisi yang sedang dipakai.
    fn persist(&self) {
        {
            let window = self.window.borrow();
            let mut cfg = self.cfg.borrow_mut();
            cfg.cam_x = window.margin(Edge::Left);
            cfg.cam_y = window.margin(Edge::Top);
        }
        Config::save_cam(&self.cfg.borrow());
    }

    fn apply_input_region(&self) {
        let window = self.window.borrow().clone();
        let Some(surface) = window.surface() else {
            return;
        };
        if self.editing.get() {
            let (w, h) = (
                self.area.content_width().max(1),
                self.area.content_height().max(1),
            );
            let region =
                gtk::cairo::Region::create_rectangle(&gtk::cairo::RectangleInt::new(0, 0, w, h));
            surface.set_input_region(&region);
        } else {
            surface.set_input_region(&gtk::cairo::Region::create());
        }
    }

    fn start_capture(&self) {
        self.stop_capture();
        let device = self.cfg.borrow().cam_device.trim().to_string();
        if device.is_empty() {
            self.report_status("Webcam: perangkat kamera belum dipilih.");
            return;
        }

        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = stop.clone();
        let frames = self.frames_tx.clone();
        let status = self.status_tx.clone();
        let adjust = self.adjust.clone();
        std::thread::spawn(move || capture_loop(device, frames, status, adjust, thread_stop));
        *self.capture.borrow_mut() = Some(Capture { stop });
    }

    fn stop_capture(&self) {
        if self.capture.borrow_mut().take().is_some() {
            *self.surface.borrow_mut() = None;
            self.area.queue_draw();
            self.report_status("Webcam: mati.");
        }
    }

    fn restart_capture(&self) {
        if self.cfg.borrow().cam {
            self.start_capture();
        }
    }
}

/// Setelan gambar yang bisa diubah dari panel tanpa membuka ulang kamera.
struct Adjust {
    brightness: AtomicI32,
    contrast: AtomicI32,
}

impl Adjust {
    fn new(brightness: i32, contrast: i32) -> Self {
        Adjust {
            brightness: AtomicI32::new(brightness.clamp(-100, 100)),
            contrast: AtomicI32::new(contrast.clamp(-100, 100)),
        }
    }
}

/// Penanda thread kamera; berhenti saat webcam dimatikan atau jendelanya diganti.
struct Capture {
    stop: Arc<AtomicBool>,
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn capture_loop(
    device: String,
    frames: async_channel::Sender<Frame>,
    status: async_channel::Sender<String>,
    adjust: Arc<Adjust>,
    stop: Arc<AtomicBool>,
) {
    let mut last_error = String::new();
    while !stop.load(Ordering::Relaxed) {
        match stream_frames(&device, &frames, &status, &adjust, &stop) {
            Ok(()) => {}
            Err(e) => {
                if e != last_error {
                    let _ = status.try_send(e.clone());
                    last_error = e;
                }
            }
        }
        // Kamera dicabut / sedang dipakai aplikasi lain: coba lagi secara berkala.
        for _ in 0..20 {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            std::thread::sleep(Duration::from_millis(150));
        }
    }
}

/// Buka kamera, minta MJPG (atau YUYV bila tidak didukung), lalu kirim frame
/// terbaru. Frame lama dibuang supaya preview tidak tertinggal.
fn stream_frames(
    device: &str,
    frames: &async_channel::Sender<Frame>,
    status: &async_channel::Sender<String>,
    adjust: &Adjust,
    stop: &AtomicBool,
) -> Result<(), String> {
    use v4l::io::traits::CaptureStream;
    use v4l::video::Capture;

    let dev = v4l::Device::with_path(device)
        .map_err(|e| format!("Webcam: gagal membuka {device} ({e})."))?;

    let mut fmt = dev
        .format()
        .map_err(|e| format!("Webcam: {device} tidak bisa dibaca ({e})."))?;
    fmt.width = CAPTURE_W;
    fmt.height = CAPTURE_H;
    fmt.fourcc = v4l::FourCC::new(b"MJPG");

    let mut fourcc = *b"MJPG";
    let fmt = match dev.set_format(&fmt) {
        Ok(f) if f.fourcc.repr == *b"MJPG" => f,
        _ => {
            let mut alt = dev
                .format()
                .map_err(|e| format!("Webcam: format kamera tidak bisa diatur ({e})."))?;
            alt.width = CAPTURE_W;
            alt.height = CAPTURE_H;
            alt.fourcc = v4l::FourCC::new(b"YUYV");
            fourcc = *b"YUYV";
            dev.set_format(&alt)
                .map_err(|e| format!("Webcam: kamera tidak mendukung MJPG/YUYV ({e})."))?
        }
    };

    let mut stream = v4l::io::mmap::Stream::with_buffers(&dev, v4l::buffer::Type::VideoCapture, 4)
        .map_err(|e| format!("Webcam: gagal memulai pengambilan gambar ({e})."))?;

    let name = device.rsplit('/').next().unwrap_or(device);
    let codec = if fourcc == *b"MJPG" { "MJPG" } else { "YUYV" };
    let _ = status.try_send(format!(
        "Kamera aktif: {name} {}×{} {codec}.",
        fmt.width, fmt.height
    ));

    while !stop.load(Ordering::Relaxed) {
        let (buf, _meta) = stream
            .next()
            .map_err(|e| format!("Webcam: frame gagal diambil ({e})."))?;
        let mut frame = decode_frame(fourcc, buf, fmt.width, fmt.height)?;
        apply_adjust(
            &mut frame.bgra,
            adjust.brightness.load(Ordering::Relaxed),
            adjust.contrast.load(Ordering::Relaxed),
        );
        // Kalau UI tertinggal, lewati frame ini supaya antreannya tidak menumpuk.
        if frames.len() < 2 {
            let _ = frames.try_send(frame);
        }
    }
    Ok(())
}

/// Rasio lebar:tinggi dari setelan; nilai tak dikenal dianggap `4:3`.
pub(crate) fn ratio_of(value: &str) -> f64 {
    match value.trim().replace(' ', "").as_str() {
        "16:9" => 16.0 / 9.0,
        "1:1" => 1.0,
        _ => 4.0 / 3.0,
    }
}

fn decode_frame(fourcc: [u8; 4], buf: &[u8], w: u32, h: u32) -> Result<Frame, String> {
    if &fourcc == b"MJPG" {
        let mut decoder = jpeg_decoder::Decoder::new(buf);
        let pixels = decoder
            .decode()
            .map_err(|e| format!("Webcam: JPEG rusak ({e})."))?;
        let info = decoder
            .info()
            .ok_or_else(|| "Webcam: info gambar kosong.".to_string())?;
        let (width, height) = (u32::from(info.width), u32::from(info.height));
        let bgra = match info.pixel_format {
            jpeg_decoder::PixelFormat::RGB24 => rgb_to_bgra(&pixels),
            jpeg_decoder::PixelFormat::L8 => gray_to_bgra(&pixels),
            other => return Err(format!("Webcam: format JPEG {other:?} tidak didukung.")),
        };
        return Ok(Frame {
            width,
            height,
            bgra,
        });
    }

    Ok(Frame {
        width: w,
        height: h,
        bgra: yuyv_to_bgra(buf, w, h)?,
    })
}

fn rgb_to_bgra(rgb: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(rgb.len() / 3 * 4);
    for px in rgb.chunks_exact(3) {
        out.extend_from_slice(&[px[2], px[1], px[0], 255]);
    }
    out
}

fn gray_to_bgra(gray: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(gray.len() * 4);
    for &y in gray {
        out.extend_from_slice(&[y, y, y, 255]);
    }
    out
}

/// YUYV 4:2:2 (2 piksel per 4 byte) → BGRA.
fn yuyv_to_bgra(buf: &[u8], w: u32, h: u32) -> Result<Vec<u8>, String> {
    let count = (w as usize) * (h as usize);
    if buf.len() < count * 2 {
        return Err("Webcam: data YUYV tidak lengkap.".to_string());
    }
    let mut out = vec![255u8; count * 4];
    for i in 0..count / 2 {
        let src = i * 4;
        let y0 = f32::from(buf[src]);
        let u = f32::from(buf[src + 1]) - 128.0;
        let y1 = f32::from(buf[src + 2]);
        let v = f32::from(buf[src + 3]) - 128.0;
        for (n, y) in [y0, y1].into_iter().enumerate() {
            let dst = (i * 2 + n) * 4;
            out[dst] = clamp_u8(y + 1.772 * u);
            out[dst + 1] = clamp_u8(y - 0.344_136 * u - 0.714_136 * v);
            out[dst + 2] = clamp_u8(y + 1.402 * v);
        }
    }
    Ok(out)
}

fn clamp_u8(v: f32) -> u8 {
    v.clamp(0.0, 255.0).round() as u8
}

/// Terapkan kecerahan & kontras ke buffer BGRA. `brightness` menggeser semua
/// kanal warna, `contrast` menjauhkan/mendekatkan nilai dari titik tengah 128.
fn apply_adjust(bgra: &mut [u8], brightness: i32, contrast: i32) {
    if brightness == 0 && contrast == 0 {
        return;
    }
    let offset = brightness.clamp(-100, 100) as f32 * 1.27;
    let factor = 1.0 + contrast.clamp(-100, 100) as f32 / 100.0;
    for px in bgra.chunks_exact_mut(4) {
        for channel in px.iter_mut().take(3) {
            let v = f32::from(*channel);
            *channel = clamp_u8((v - 128.0) * factor + 128.0 + offset);
        }
    }
}

fn draw(ui: &CamUi, cr: &gtk::cairo::Context, w: i32, h: i32) {
    let (wf, hf) = (f64::from(w), f64::from(h));
    if wf < 2.0 || hf < 2.0 {
        return;
    }
    let radius = 14.0_f64.min(wf / 2.0).min(hf / 2.0);

    // Latar gelap supaya bingkai tetap terlihat saat kamera belum mengirim gambar.
    rounded_path(cr, 0.5, 0.5, wf - 1.0, hf - 1.0, radius);
    cr.set_source_rgba(0.05, 0.06, 0.08, 0.85);
    let _ = cr.fill();

    if let Some(surface) = ui.surface.borrow().as_ref() {
        rounded_path(cr, 0.5, 0.5, wf - 1.0, hf - 1.0, radius);
        cr.clip();

        let (sw, sh) = (f64::from(surface.width()), f64::from(surface.height()));
        if sw > 0.0 && sh > 0.0 {
            let scale = (wf / sw).max(hf / sh);
            let mirror = ui.cfg.borrow().cam_mirror;
            cr.save().ok();
            if mirror {
                cr.translate(wf, 0.0);
                cr.scale(-1.0, 1.0);
            }
            cr.translate((wf - sw * scale) / 2.0, (hf - sh * scale) / 2.0);
            cr.scale(scale, scale);
            let _ = cr.set_source_surface(surface, 0.0, 0.0);
            let _ = cr.paint();
            cr.restore().ok();
        }
        cr.reset_clip();
    }

    // Bingkai tipis; berwarna saat mode edit supaya jendelanya mudah dikenali.
    let editing = ui.editing.get();
    rounded_path(cr, 1.0, 1.0, wf - 2.0, hf - 2.0, radius - 0.5);
    if editing {
        cr.set_source_rgba(0.36, 0.66, 1.0, 0.95);
        cr.set_line_width(2.0);
    } else {
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.18);
        cr.set_line_width(1.0);
    }
    let _ = cr.stroke();
}

fn rounded_path(cr: &gtk::cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    use std::f64::consts::{FRAC_PI_2, PI};
    let r = r.max(0.0).min(w / 2.0).min(h / 2.0);
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -FRAC_PI_2, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, FRAC_PI_2);
    cr.arc(x + r, y + h - r, r, FRAC_PI_2, PI);
    cr.arc(x + r, y + r, r, PI, 3.0 * FRAC_PI_2);
    cr.close_path();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yuyv_menghasilkan_satu_piksel_per_dua_byte() {
        // 2×2 piksel: satu blok YUYV (Y0 U Y1 V) per baris.
        let buf = [80u8, 128, 80, 128, 80, 128, 80, 128];
        let out = yuyv_to_bgra(&buf, 2, 2).expect("konversi");
        assert_eq!(out.len(), 2 * 2 * 4);
        // U/V netral → abu-abu sesuai nilai Y, alpha penuh.
        assert_eq!(&out[..4], &[80, 80, 80, 255]);
    }

    #[test]
    fn yuyv_pendek_ditolak() {
        assert!(yuyv_to_bgra(&[0, 0, 0, 0], 4, 4).is_err());
    }

    #[test]
    fn kecerahan_dan_kontras_diterapkan() {
        let asli = [100u8, 100, 100, 255, 200, 200, 200, 128];

        let mut px = asli;
        apply_adjust(&mut px, 0, 0);
        assert_eq!(px, asli, "tanpa setelan gambarnya tidak berubah");

        let mut px = asli;
        apply_adjust(&mut px, 50, 0);
        assert_eq!(&px[..4], &[164, 164, 164, 255]);
        assert_eq!(&px[4..7], &[255, 255, 255], "nilai di atas 255 dibatasi");

        let mut px = asli;
        apply_adjust(&mut px, 0, 100);
        assert_eq!(&px[..3], &[72, 72, 72], "kontras menjauh dari 128");
        assert_eq!(&px[4..7], &[255, 255, 255]);
        assert_eq!(px[7], 128, "alpha tidak ikut diubah");

        let mut px = asli;
        apply_adjust(&mut px, -100, -100);
        assert_eq!(&px[..3], &[1, 1, 1], "kontras minimum meratakan warna");
    }

    #[test]
    fn rasio_webcam_dikenali() {
        assert!((ratio_of("16:9") - 1.777_8).abs() < 0.001);
        assert!((ratio_of(" 1:1 ") - 1.0).abs() < f64::EPSILON);
        assert!((ratio_of("4:3") - 1.333_3).abs() < 0.001);
        // Nilai aneh tetap jatuh ke rasio kamera bawaan, bukan membagi nol.
        assert!((ratio_of("9:16x") - 4.0 / 3.0).abs() < f64::EPSILON);
    }
}

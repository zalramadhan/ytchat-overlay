mod config;
mod control;
mod ipc;
mod model;
mod obs;
mod overlay;
mod webcam;
mod youtube;

use config::Config;
use model::{Ctl, Event, JobRequest};
use std::process::ExitCode;

fn print_help() {
    println!(
        "ytchat-overlay — overlay chat YouTube live (transparan, klik-tembus)\n\
         \n\
         Tanpa argumen: buka panel aplikasi (tempel link, atur posisi, mulai overlay).\n\
         \x20 ytchat-overlay [link-atau-id-video]   langsung mulai overlay untuk live itu\n\
         \x20 ytchat-overlay ctl <perintah>          kendalikan overlay dari terminal\n\
         \n\
         PERINTAH ctl:\n\
         \x20 toggle   sembunyikan / tampilkan overlay\n\
         \x20 edit     masuk/keluar mode edit (geser posisi dengan mouse)\n\
         \x20 lock     keluar mode edit & simpan posisi\n\
         \x20 clear    hapus semua pesan\n\
         \x20 quit     keluar dari aplikasi\n\
         \n\
         API key dibaca dari env YOUTUBE_API_KEY atau `api_key` di\n\
         {}",
        Config::path().display()
    );
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    match args.first().map(String::as_str) {
        Some("-h") | Some("--help") => {
            print_help();
            return ExitCode::SUCCESS;
        }
        Some("ctl") => {
            let Some(cmd) = args.get(1) else {
                eprintln!("Pakai: ytchat-overlay ctl <toggle|edit|lock|clear|quit>");
                return ExitCode::FAILURE;
            };
            if Ctl::parse(cmd).is_none() {
                eprintln!("Perintah tidak dikenal: {cmd}");
                return ExitCode::FAILURE;
            }
            return match ipc::send(cmd) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("Gagal menghubungi overlay: {e} (aplikasi belum jalan?)");
                    ExitCode::FAILURE
                }
            };
        }
        _ => {}
    }

    let mut cfg = Config::load();
    if let Ok(key) = std::env::var("YOUTUBE_API_KEY") {
        if !key.trim().is_empty() {
            cfg.api_key = key;
        }
    }
    let start_video = args.iter().find(|a| !a.starts_with('-')).cloned();
    if let Some(video) = &start_video {
        cfg.video = video.clone();
    }

    let (tx, rx) = async_channel::unbounded::<Event>();
    // Event yang sama dipakai dua tampilan: overlay di layar dan halaman OBS.
    let (tx_ui, rx_ui) = async_channel::unbounded::<Event>();
    let (job_tx, job_rx) = async_channel::unbounded::<JobRequest>();

    let obs_hub = obs::Hub::new(&cfg);
    let obs_ctl = obs::ObsCtl::spawn(obs_hub.clone(), obs::DEFAULT_PORT);
    let base_cfg = cfg.clone();
    let hub_tee = obs_hub.clone();
    std::thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => {
                eprintln!("Gagal membuat runtime async: {e}");
                return;
            }
        };
        rt.block_on(async move {
            let tx_ipc = tx.clone();
            tokio::spawn(async move { ipc::serve(tx_ipc).await });

            tokio::spawn(async move {
                while let Ok(ev) = rx.recv().await {
                    hub_tee.publish(&ev);
                    if tx_ui.send(ev).await.is_err() {
                        break;
                    }
                }
            });

            // Satu job aktif pada satu waktu: permintaan baru membatalkan yang lama.
            let mut job: Option<tokio::task::JoinHandle<()>> = None;
            while let Ok(req) = job_rx.recv().await {
                if let Some(old) = job.take() {
                    old.abort();
                }
                let mut job_cfg = base_cfg.clone();
                job_cfg.video = req.video;
                job_cfg.api_key = req.api_key;
                let tx_job = tx.clone();
                job = Some(tokio::spawn(async move { youtube::run(job_cfg, tx_job).await }));
            }

            // Tetap hidupkan runtime + server kontrol walau pengambil chat berhenti
            // (mis. API key kosong atau live sudah berakhir), supaya `ctl` masih jalan.
            std::future::pending::<()>().await;
        });
    });

    overlay::run(cfg, rx_ui, job_tx, start_video, obs_hub, obs_ctl);
    ExitCode::SUCCESS
}

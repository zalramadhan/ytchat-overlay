use crate::model::{Ctl, Event};
use async_channel::Sender;
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::UnixListener;

pub fn sock_path() -> PathBuf {
    let base = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(base).join("ytchat-overlay.sock")
}

/// Server kontrol: terima perintah "toggle|edit|lock|clear|quit" dari `ytchat-overlay ctl <cmd>`.
pub async fn serve(tx: Sender<Event>) {
    let path = sock_path();
    // Jangan hapus socket milik instance lain yang masih melayani: cukup mundur.
    if std::os::unix::net::UnixStream::connect(&path).is_ok() {
        return;
    }
    let _ = std::fs::remove_file(&path);
    let Ok(listener) = UnixListener::bind(&path) else {
        eprintln!("Gagal bind socket kontrol: {}", path.display());
        return;
    };
    loop {
        let Ok((stream, _)) = listener.accept().await else {
            continue;
        };
        let tx = tx.clone();
        tokio::spawn(async move {
            let mut line = String::new();
            let mut r = BufReader::new(stream);
            if r.read_line(&mut line).await.is_ok() {
                if let Some(c) = Ctl::parse(line.trim()) {
                    let _ = tx.send(Event::Ctl(c)).await;
                }
            }
        });
    }
}

/// Sisi klien (sinkron).
pub fn send(cmd: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut s = std::os::unix::net::UnixStream::connect(sock_path())?;
    s.write_all(format!("{cmd}\n").as_bytes())
}

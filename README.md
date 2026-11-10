# ytchat-overlay

Overlay chat YouTube Live untuk **KDE Plasma 6 (Wayland)**: jendela transparan
yang selalu di atas game, **klik-tembus** (klik tetap tembus ke game), dan
posisinya bisa digeser dengan mouse saat mode edit.

Isi overlay: nama pengirim + badge (owner/mod/member), avatar, emoji (termasuk
emoji custom kanal), highlight Super Chat/Super Sticker + nominal, dan pesan
lama memudar otomatis.

## Kebutuhan

- KDE Plasma 6 dengan sesi **Wayland** (butuh protokol `wlr-layer-shell`).
- `gtk4-layer-shell` (skrip pasang menangani ini).
- Rust/cargo untuk build.
- YouTube Data API v3 key (dari Google Cloud Console, aktifkan *YouTube Data API v3*).

## Pasang

```bash
./install.sh
```

Skrip itu: memasang `gtk4-layer-shell` bila belum ada (minta password sudo),
build release, memasang binary ke `~/.local/bin`, menambah entri menu
"Chat YouTube Live" + ikon, lalu langsung menjalankan aplikasinya.

Opsi lain:

```bash
./install.sh --no-run      # pasang saja, tanpa menjalankan
./install.sh --no-deps     # lewati pemasangan dependensi sistem
./install.sh --uninstall   # hapus binary, entri menu, dan ikon
```

Setelah terpasang, aplikasi bisa dibuka dari menu aplikasi KDE kapan saja.
`--uninstall` menghentikan aplikasi yang sedang berjalan dan membiarkan file
config (`~/.config/ytchat-overlay/`) tetap ada.

## Pemakaian panel

1. Buka **Chat YouTube Live** dari menu.
2. Tempel **link live** (bentuk `https://www.youtube.com/watch?v=…`,
   `https://youtu.be/…`, atau `https://www.youtube.com/live/…`) — atau cukup ID videonya.
3. Isi **API key** (tersimpan di config; cukup sekali).
4. Tekan **Tampilkan overlay**. Chat akan muncul di pojok kiri atas layar.

Tombol di panel:

| Tombol | Fungsi |
|---|---|
| **Tampilkan overlay** | Menyambung ke live tersebut dan menampilkan overlay. Ganti link lalu tekan lagi untuk pindah live. |
| **Mode edit posisi** | Mengaktifkan input pada overlay: geser dengan mouse, lalu matikan lagi untuk menyimpan posisi. |
| **Sembunyikan / Tampilkan** | Menyembunyikan overlay tanpa memutus chat. |
| **Bersihkan chat** | Menghapus semua pesan yang sedang tampil. |
| **Keluar** | Menutup aplikasi (overlay ikut hilang). |

Menutup jendela panel juga menghentikan overlay, supaya tidak ada overlay
"hantu" yang tidak bisa dikendalikan lagi.

### Bagian Tampilan

| Kontrol | Arti |
|---|---|
| **Opacity latar** | Kegelapan kotak di belakang tiap pesan (0% = tanpa latar, 100% = hitam pekat). Berlaku langsung ke pesan yang sudah tampil. |
| **Ukuran font** | Ukuran teks chat dalam piksel (10–48). |
| **Monitor** | Layar tempat overlay tampil. "Ikuti layar aktif" = layar yang sedang dipakai. Pindah monitor tidak menghapus chat yang sudah tampil. |

Semua setelan di panel tersimpan otomatis ke file config.

## File config

`~/.config/ytchat-overlay/config.toml` (dibuat otomatis saat pertama jalan).
Bisa diedit manual; perubahan dari file berlaku setelah aplikasi dijalankan ulang.

| Kunci | Default | Arti |
|---|---|---|
| `api_key` | `""` | YouTube Data API v3 key. Bisa juga lewat env `YOUTUBE_API_KEY`. |
| `video` | `""` | Link/ID live terakhir (diisi otomatis dari panel). |
| `output` | `""` | Nama konektor monitor, mis. `HDMI-A-1` / `DP-1`. Kosong = layar aktif. |
| `layer` | `"overlay"` | Lapisan layer-shell: `overlay` (paling atas) atau `top`. |
| `x`, `y` | `30`, `200` | Posisi kotak chat (piksel dari pojok kiri atas monitor). |
| `width`, `height` | `420`, `500` | Ukuran kotak chat. |
| `font_family` | `"Noto Sans"` | Font chat. |
| `font_size` | `16.0` | Ukuran font (piksel). |
| `text_color` | `"#ffffff"` | Warna teks chat. |
| `bg_opacity` | `0.25` | Kegelapan latar pesan (0.0–1.0). |
| `show_avatars` | `true` | Tampilkan avatar pengirim. |
| `avatar_size` | `28` | Ukuran avatar (piksel). |
| `max_messages` | `40` | Jumlah pesan maksimum yang ditahan di layar. |
| `fade_after_secs` | `30.0` | Pesan mulai memudar setelah N detik. `0` = tidak pernah memudar. |
| `fade_duration_secs` | `3.0` | Lama proses memudar sampai hilang. |
| `poll_min_ms` | `5000` | Interval polling minimum ke API (ms). Lebih besar = lebih hemat kuota. |

## Pemakaian lewat terminal (opsional)

```bash
ytchat-overlay "https://youtu.be/VIDEO_ID"   # langsung jalankan overlay + panel
ytchat-overlay ctl toggle                    # sembunyikan/tampilkan overlay
ytchat-overlay ctl edit                      # masuk/keluar mode edit posisi
ytchat-overlay ctl lock                      # keluar mode edit & simpan posisi
ytchat-overlay ctl clear                     # hapus semua pesan
ytchat-overlay ctl quit                      # keluar dari aplikasi
```

## Catatan

- Kuota API gratis 10.000 unit/hari; polling tiap 5 detik biasanya cukup untuk
  beberapa jam pemakaian.
- Emoji custom diambil sekali dari halaman live chat. Kalau peta emoji tidak
  tersedia, emoji custom tampil sebagai teks `:nama:`.
- Kalau ikon belum muncul di menu setelah install: jalankan `kbuildsycoca6`
  atau logout/login sekali.
- Uninstall: `./install.sh --uninstall` (config dan setelan tidak dihapus).

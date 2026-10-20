//! hidraw のデバイス一覧、レポート記述子の表示、入力レポートのライブ解析。
//!
//!   hid-probe list            デバイス一覧（名前・VID:PID・記述子サイズ）
//!   hid-probe desc <N>        /dev/hidrawN のレポート記述子を hex で出す
//!   hid-probe watch <N>       入力レポートを hex + 解釈付きで流す
//!   hid-probe bridge <SERIAL> <KBD_N> [<MEDIA_N>]
//!                             キーボード(IF0)とマウス/メディア(IF2)のレポートを
//!                             ESP の USB シリアルへ中継する（暫定の USB ホスト役）
//!   hid-probe evbridge <SERIAL> [<EVDEV>]
//!                             Linux の入力イベント（evdev。既定は keyd の仮想キーボード）を
//!                             ブートキーボードのレポートにして S3 へ送る（S3 は `serial-bridge` の版）。
//!                             Esc を 200ms 以上押すと F24（hikyaku のスクロール切り替え）になる。

use hikyaku_hid::{usage_name, BootKeyboardReport, MOD_NAMES};
use std::{
    fs,
    io::{Read, Write},
    sync::{Arc, Mutex},
    thread,
    time::Instant,
};

fn sysfs(n: &str, file: &str) -> String {
    fs::read_to_string(format!("/sys/class/hidraw/hidraw{n}/device/{file}")).unwrap_or_default()
}

fn uevent_field(n: &str, key: &str) -> String {
    sysfs(n, "uevent")
        .lines()
        .find_map(|l| l.strip_prefix(&format!("{key}=")))
        .unwrap_or("?")
        .to_string()
}

fn descriptor(n: &str) -> Vec<u8> {
    fs::read(format!("/sys/class/hidraw/hidraw{n}/device/report_descriptor")).unwrap_or_default()
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect::<Vec<_>>().join(" ")
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("list") => {
            let mut ents: Vec<_> = fs::read_dir("/sys/class/hidraw").unwrap().flatten().collect();
            ents.sort_by_key(|e| e.file_name());
            for e in ents {
                let n = e.file_name().to_string_lossy().trim_start_matches("hidraw").to_string();
                println!(
                    "hidraw{n}\t{}\t{}\tdesc={}B",
                    uevent_field(&n, "HID_ID"),
                    uevent_field(&n, "HID_NAME"),
                    descriptor(&n).len()
                );
            }
        }
        Some("desc") => println!("{}", hex(&descriptor(args.get(2).expect("N"))))
        ,
        Some("watch") => {
            let n = args.get(2).expect("N");
            let mut f = fs::File::open(format!("/dev/hidraw{n}"))
                .expect("open 失敗。権限が必要（sudo setfacl -m u:$USER:r /dev/hidrawN）");
            let start = Instant::now();
            let mut buf = [0u8; 64];
            loop {
                let len = f.read(&mut buf).expect("read");
                let raw = &buf[..len];
                let t = start.elapsed().as_secs_f64();
                print!("[{t:8.3}] {len}B: {}", hex(raw));
                // 記述子にレポート ID がある場合（先頭 1 バイトが ID）は簡易解釈。
                match (raw.first(), len) {
                    (Some(2), 6) => print!("  mouse btn={:#04x} dx={} dy={} wheel={} pan={}", raw[1], raw[2] as i8, raw[3] as i8, raw[4] as i8, raw[5] as i8),
                    (Some(3), 3) => print!("  system usage={:#06x}", u16::from_le_bytes([raw[1], raw[2]])),
                    (Some(4), 3) => print!("  consumer usage={:#06x}", u16::from_le_bytes([raw[1], raw[2]])),
                    _ => {
                        if let Some(r) = BootKeyboardReport::parse(raw) {
                            let mods: Vec<_> = (0..8).filter(|i| r.modifiers >> i & 1 == 1).map(|i| MOD_NAMES[i]).collect();
                            let keys: Vec<_> = r.pressed().map(|k| format!("{}(0x{k:02x})", usage_name(k))).collect();
                            print!("  mods={mods:?} keys={keys:?}");
                            if r.is_rollover_error() {
                                print!("  [RollOver]");
                            }
                        }
                    }
                }
                println!();
            }
        }
        Some("bridge") => bridge(&args[2..]),
        Some("evbridge") => evbridge(&args[2..]),
        _ => eprintln!("usage: hid-probe list | desc <N> | watch <N> | bridge <SERIAL> <KBD_N> [<MEDIA_N>] | evbridge <SERIAL> [<EVDEV>]"),
    }
}

/// hidraw のレポートを ESP へ行形式で送る。
/// K<hex8B> キーボード / M<hex5B> マウス / C<hex2B> コンシューマ
fn bridge(args: &[String]) {
    let serial_path = args.first().expect("SERIAL (例: /dev/ttyACM0)");
    let kbd = args.get(1).expect("KBD_N");
    let media = args.get(2);

    // 行末変換やエコーを切る。close で DTR が落ちて ESP がリセットされないよう -hupcl。
    // 開いた直後に DTR/RTS を同時に落とす（USB-Serial-JTAG は DTR/RTS の遷移で ESP をリセットするため、
    // stty など別プロセスで開閉せず、ここで 1 回だけ開く）。
    let port = fs::OpenOptions::new().read(true).write(true).open(serial_path).expect("serial open");
    unsafe {
        use std::os::fd::AsRawFd;
        let fd = port.as_raw_fd();
        let mut t: libc::termios = std::mem::zeroed();
        libc::tcgetattr(fd, &mut t);
        libc::cfmakeraw(&mut t);
        t.c_cflag &= !libc::HUPCL;
        libc::tcsetattr(fd, libc::TCSANOW, &t);
        let bits: libc::c_int = libc::TIOCM_DTR | libc::TIOCM_RTS;
        libc::ioctl(fd, libc::TIOCMBIC, &bits);
    }
    // ESP のログを流す（何が起きているか見えるように）。
    let mut log_rx = port.try_clone().expect("clone serial");
    thread::spawn(move || {
        let mut buf = [0u8; 256];
        while let Ok(n) = log_rx.read(&mut buf) {
            let _ = std::io::stderr().write_all(&buf[..n]);
        }
    });
    let port = Arc::new(Mutex::new(port));

    let send = {
        let port = port.clone();
        move |line: String| {
            let mut p = port.lock().unwrap();
            let _ = p.write_all(line.as_bytes());
            let _ = p.write_all(b"\n");
        }
    };

    let mut handles = Vec::new();
    {
        let send = send.clone();
        let path = format!("/dev/hidraw{kbd}");
        handles.push(thread::spawn(move || {
            let mut f = fs::File::open(&path).expect("kbd open");
            let mut buf = [0u8; 64];
            while let Ok(n) = f.read(&mut buf) {
                if n == 8 {
                    send(format!("K{}", hex_compact(&buf[..8])));
                }
            }
        }));
    }
    if let Some(m) = media {
        let path = format!("/dev/hidraw{m}");
        handles.push(thread::spawn(move || {
            let mut f = fs::File::open(&path).expect("media open");
            let mut buf = [0u8; 64];
            while let Ok(n) = f.read(&mut buf) {
                match (buf[0], n) {
                    (2, 6) => send(format!("M{}", hex_compact(&buf[1..6]))),
                    (4, 3) => send(format!("C{}", hex_compact(&buf[1..3]))),
                    _ => {}
                }
            }
        }));
    }
    eprintln!("bridging -> {serial_path} (Ctrl-C で終了)");
    for h in handles {
        let _ = h.join();
    }
}

fn hex_compact(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Linux のキーコード（`linux/input-event-codes.h`）→ HID の Usage ID。修飾キーは `Err(ビット)`。
fn linux_to_hid(code: u16) -> Option<Result<u8, u8>> {
    const LETTERS_ROW: [(u16, u8); 26] = [
        (30, 0x04), (48, 0x05), (46, 0x06), (32, 0x07), (18, 0x08), (33, 0x09), (34, 0x0A), (35, 0x0B), (23, 0x0C),
        (36, 0x0D), (37, 0x0E), (38, 0x0F), (50, 0x10), (49, 0x11), (24, 0x12), (25, 0x13), (16, 0x14), (19, 0x15),
        (31, 0x16), (20, 0x17), (22, 0x18), (47, 0x19), (17, 0x1A), (45, 0x1B), (21, 0x1C), (44, 0x1D),
    ];
    if let Some(&(_, u)) = LETTERS_ROW.iter().find(|(c, _)| *c == code) {
        return Some(Ok(u));
    }
    let u = match code {
        2..=10 => 0x1E + (code - 2) as u8, // 1〜9
        11 => 0x27,                        // 0
        1 => 0x29,                         // Esc
        12 => 0x2D,
        13 => 0x2E,
        14 => 0x2A, // Backspace
        15 => 0x2B, // Tab
        26 => 0x2F,
        27 => 0x30,
        28 => 0x28, // Enter
        39 => 0x33,
        40 => 0x34,
        41 => 0x35,
        43 => 0x31,
        51 => 0x36,
        52 => 0x37,
        53 => 0x38,
        57 => 0x2C, // Space
        58 => 0x39, // CapsLock
        59..=68 => 0x3A + (code - 59) as u8, // F1〜F10
        87 => 0x44,
        88 => 0x45,
        183..=194 => 0x68 + (code - 183) as u8, // F13〜F24
        102 => 0x4A,
        103 => 0x52,
        104 => 0x4B,
        105 => 0x50,
        106 => 0x4F,
        107 => 0x4D,
        108 => 0x51,
        109 => 0x4E,
        110 => 0x49,
        111 => 0x4C,
        85 => 0x35,  // 全角/半角
        86 => 0x64,  // 102nd
        89 => 0x87,  // ろ
        92 => 0x8A,  // 変換
        93 => 0x88,  // かな
        94 => 0x8B,  // 無変換
        124 => 0x89, // ￥
        29 => return Some(Err(0)),  // LCtrl
        42 => return Some(Err(1)),  // LShift
        56 => return Some(Err(2)),  // LAlt
        125 => return Some(Err(3)), // LMeta
        97 => return Some(Err(4)),  // RCtrl
        54 => return Some(Err(5)),  // RShift
        100 => return Some(Err(6)), // RAlt
        126 => return Some(Err(7)), // RMeta
        _ => return None,
    };
    Some(Ok(u))
}

/// evdev のキー入力をブートキーボードのレポートにして、S3 の USB シリアルへ `K<16 桁 hex>` で送る。
/// Esc は「押して 200ms 以内に離したら Esc、200ms 以上押していたら F24（離すまで）」にする（QMK のモッドタップの代わり）。
/// 入力はデスクトップにも届く（奪わない）ので、試すときは何も起きないウィンドウにフォーカスしておく。
fn evbridge(args: &[String]) {
    use std::os::fd::AsRawFd;
    let serial_path = args.first().expect("SERIAL (例: /dev/ttyACM0)");
    let dev = args.get(1).cloned().unwrap_or_else(|| {
        // keyd が内蔵キーボードを奪っているので、keyd の仮想キーボードを読む
        let list = fs::read_to_string("/proc/bus/input/devices").unwrap_or_default();
        let mut found = None;
        for block in list.split("\n\n") {
            if block.contains("keyd virtual keyboard") {
                found = block
                    .lines()
                    .find(|l| l.starts_with("H: Handlers="))
                    .and_then(|l| l.split_whitespace().find(|w| w.starts_with("event")).map(|e| format!("/dev/input/{e}")));
            }
        }
        found.expect("keyd の仮想キーボードが見つからない。EVDEV を指定する（例: /dev/input/event3）")
    });

    let port = fs::OpenOptions::new().read(true).write(true).open(serial_path).expect("serial open");
    unsafe {
        let fd = port.as_raw_fd();
        let mut t: libc::termios = std::mem::zeroed();
        libc::tcgetattr(fd, &mut t);
        libc::cfmakeraw(&mut t);
        t.c_cflag &= !libc::HUPCL;
        libc::tcsetattr(fd, libc::TCSANOW, &t);
    }
    let mut log_rx = port.try_clone().expect("clone serial");
    thread::spawn(move || {
        let mut buf = [0u8; 256];
        while let Ok(n) = log_rx.read(&mut buf) {
            let _ = std::io::stderr().write_all(&buf[..n]);
        }
    });
    let mut port = port;

    let mut input = fs::File::open(&dev).expect("evdev open（input グループか ACL が必要）");
    eprintln!("evbridge: {dev} -> {serial_path}（Esc 長押し = F24。Ctrl-C で終了）");

    const ESC: u8 = 0x29;
    const F24: u8 = 0x73;
    const HOLD_MS: u128 = 200;
    let mut mods = 0u8;
    let mut keys: Vec<u8> = Vec::new();
    let mut esc_down: Option<Instant> = None; // Esc を押して、まだ Esc か F24 か決まっていない
    let mut f24 = false;

    let send = |mods: u8, keys: &[u8], port: &mut fs::File| {
        let mut r = [0u8; 8];
        r[0] = mods;
        for (d, k) in r[2..].iter_mut().zip(keys.iter()) {
            *d = *k;
        }
        let _ = port.write_all(format!("K{}\n", hex_compact(&r)).as_bytes());
    };

    let mut ev = [0u8; 24]; // struct input_event（64 ビット Linux）
    loop {
        // Esc の長押し判定のため、200ms で一度起きる
        let mut pfd = libc::pollfd { fd: input.as_raw_fd(), events: libc::POLLIN, revents: 0 };
        let timeout = match esc_down {
            Some(t) => (HOLD_MS.saturating_sub(t.elapsed().as_millis())) as i32,
            None => -1,
        };
        let r = unsafe { libc::poll(&mut pfd, 1, timeout) };
        if r == 0 {
            // 長押し: F24 にする
            if esc_down.take().is_some() {
                f24 = true;
                keys.push(F24);
                send(mods, &keys, &mut port);
                eprintln!("[F24 on]");
            }
            continue;
        }
        if input.read_exact(&mut ev).is_err() {
            break;
        }
        let ty = u16::from_ne_bytes([ev[16], ev[17]]);
        let code = u16::from_ne_bytes([ev[18], ev[19]]);
        let value = i32::from_ne_bytes([ev[20], ev[21], ev[22], ev[23]]);
        if ty != 1 || value == 2 {
            continue; // EV_KEY 以外、キーリピートは無視
        }
        let Some(m) = linux_to_hid(code) else { continue };
        match (m, value) {
            (Ok(ESC), 1) => {
                esc_down = Some(Instant::now());
                continue;
            }
            (Ok(ESC), 0) => {
                if esc_down.take().is_some() {
                    // 短く押した: Esc を押して離す
                    keys.push(ESC);
                    send(mods, &keys, &mut port);
                    keys.retain(|&k| k != ESC);
                } else if f24 {
                    f24 = false;
                    keys.retain(|&k| k != F24);
                    eprintln!("[F24 off]");
                }
            }
            (Ok(u), 1) => {
                if !keys.contains(&u) && keys.len() < 6 {
                    keys.push(u);
                }
            }
            (Ok(u), 0) => keys.retain(|&k| k != u),
            (Err(bit), 1) => mods |= 1 << bit,
            (Err(bit), 0) => mods &= !(1 << bit),
            _ => continue,
        }
        send(mods, &keys, &mut port);
    }
}

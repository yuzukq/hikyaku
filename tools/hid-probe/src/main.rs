//! hidraw のデバイス一覧、レポート記述子の表示、入力レポートのライブ解析。
//!
//!   hid-probe list            デバイス一覧（名前・VID:PID・記述子サイズ）
//!   hid-probe desc <N>        /dev/hidrawN のレポート記述子を hex で出す
//!   hid-probe watch <N>       入力レポートを hex + 解釈付きで流す
//!   hid-probe bridge <SERIAL> <KBD_N> [<MEDIA_N>]
//!                             キーボード(IF0)とマウス/メディア(IF2)のレポートを
//!                             ESP の USB シリアルへ中継する（暫定の USB ホスト役）

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
        _ => eprintln!("usage: hid-probe list | desc <N> | watch <N> | bridge <SERIAL> <KBD_N> [<MEDIA_N>]"),
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
    let port = fs::OpenOptions::new().write(true).open(serial_path).expect("serial open");
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

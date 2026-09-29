//! hidraw のデバイス一覧、レポート記述子の表示、入力レポートのライブ解析。
//!
//!   hid-probe list            デバイス一覧（名前・VID:PID・記述子サイズ）
//!   hid-probe desc <N>        /dev/hidrawN のレポート記述子を hex で出す
//!   hid-probe watch <N>       入力レポートを hex + 解釈付きで流す

use hikyaku_hid::{usage_name, BootKeyboardReport, MOD_NAMES};
use std::{fs, io::Read, time::Instant};

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
                if let Some(r) = BootKeyboardReport::parse(raw) {
                    let mods: Vec<_> = (0..8).filter(|i| r.modifiers >> i & 1 == 1).map(|i| MOD_NAMES[i]).collect();
                    let keys: Vec<_> = r.pressed().map(|k| format!("{}(0x{k:02x})", usage_name(k))).collect();
                    print!("  mods={mods:?} keys={keys:?}");
                    if r.is_rollover_error() {
                        print!("  [RollOver]");
                    }
                }
                println!();
            }
        }
        _ => eprintln!("usage: hid-probe list | desc <N> | watch <N>"),
    }
}

//! 実機のトラックパッドの生データ（`--features trackpad-trace` のログ）を再生して、
//! `hikyaku-pointer` の出力を確認する。設定値の調整用。
//!
//! 使い方:
//!   python3 tools/scripts/uart_cap.py 60 > trace.log      # 実機で触りながらログを取る
//!   cargo run -q -p hikyaku-pointer --example replay -- trace.log [gain=90 start=10 ...]
//!
//! 入力: `tp <ms> <x> <y> <z>` を含む行（他の行は無視）。z=0 は指が離れている。
//! 出力: 出したレポートの一覧と、移動量の合計・ジャンプ・タップ数などの集計。

use hikyaku_pointer::{Config, Pointer};
use std::{env, fs};

fn main() {
    let mut args = env::args().skip(1);
    let path = args.next().expect("trace file");
    let mut cfg = Config::default();
    for kv in args {
        let (k, v) = kv.split_once('=').expect("key=value");
        let n: i64 = v.parse().expect("number");
        match k {
            "gain" => cfg.gain_q8 = n as u32,
            "accel_ref" => cfg.accel_ref = n as u32,
            "accel_max" => cfg.accel_max_q8 = n as u32,
            "smooth_max" => cfg.smoothing_q8 = n as i32,
            "smooth_min" => cfg.smooth_min_q8 = n as i32,
            "smooth_speed" => cfg.smooth_full_speed = n as i32,
            "start" => cfg.start_threshold = n as i32,
            "damp_ratio" => cfg.axis_damp_ratio = n as i32,
            "damp" => cfg.axis_damp_q8 = n as i32,
            "tap_ms" => cfg.tap_max_ms = n as u32,
            "tap_move" => cfg.tap_max_move = n as i32,
            "median" => cfg.median3 = n != 0,
            "invert_y" => cfg.invert_y = n != 0,
            "invert_x" => cfg.invert_x = n != 0,
            _ => panic!("unknown key {k}"),
        }
    }
    let text = fs::read_to_string(&path).expect("read");
    let mut p = Pointer::new(cfg);
    let (mut sx, mut sy, mut n, mut clicks) = (0i64, 0i64, 0usize, 0usize);
    let (mut max_step, mut samples, mut touches) = (0i32, 0usize, 0usize);
    let mut was_touch = false;
    let mut prev_buttons = 0u8;
    for line in text.lines() {
        let Some(i) = line.find("tp ") else { continue };
        let f: Vec<&str> = line[i + 3..].split_whitespace().collect();
        if f.len() < 4 {
            continue;
        }
        let (Ok(t), Ok(x), Ok(y), Ok(z)) = (f[0].parse::<u32>(), f[1].parse::<u16>(), f[2].parse::<u16>(), f[3].parse::<u8>())
        else {
            continue;
        };
        samples += 1;
        let touching = z > 0;
        if touching && !was_touch {
            touches += 1;
        }
        was_touch = touching;
        if let Some(r) = p.update(t, touching.then_some((x, y)), false) {
            n += 1;
            sx += r.x as i64;
            sy += r.y as i64;
            max_step = max_step.max((r.x as i32).abs()).max((r.y as i32).abs());
            if r.buttons & 1 != 0 && prev_buttons & 1 == 0 {
                clicks += 1;
            }
            prev_buttons = r.buttons;
            println!("{t:>8} raw=({x:>4},{y:>4},{z:>2})  -> btn={} x={:>4} y={:>4} wheel={} pan={}", r.buttons, r.x, r.y, r.wheel, r.pan);
        }
    }
    println!("--");
    println!("samples={samples} touches={touches} reports={n} clicks={clicks}");
    println!("sum dx={sx} dy={sy}  max single step={max_step}");
}

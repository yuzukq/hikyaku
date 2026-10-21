//! 実機の相対モードのログ（`--features trackpad-trace` の `rel <ms> raw=[..]` 行）を再生して、
//! ファームと同じ変換（`hikyaku_pointer::relative::Relay`）が出すマウスレポートを確かめる。
//!
//! 使い方:
//!   cargo run -q -p hikyaku-pointer --example replay_rel -- tools/traces/2026-10-13-relative-all.log [key=value ...]
//!
//! キー（省略時はファームの既定値）:
//!   gain=9 gain_div=5 swap=1 invx=1 invy=1 scroll_div=18 scroll_invert=1   変換の設定
//!   still=0                                    1 で静止中のぶれを抑えるフィルタ（trackpad-still-filter）を有効にする
//!   tap=0                                      1 で自前のタップ判定（trackpad-soft-tap）を有効にする（パッドのタップビットは無視）
//!   scroll=0                                   1 でスクロールモードとして再生
//!   quiet=1                                    1 行ごとの出力を省き、集計だけ出す
//!   skip_fc=1                                  配線が外れているときのゴミ（raw=[fc, fc, fc]）を読み飛ばす
//!
//! 出力: 1 パケット 1 行（時刻・生バイト・dx/dy・出たレポート）と、末尾に集計
//! （パケット数、ゼロパケット数、レポート数、移動量の合計、1 レポートの最大移動量、クリック数）。

use hikyaku_pointer::relative::{parse_trace_line, Config, Relay, SoftTap, StillFilter};
use std::{env, fs};

fn main() {
    let mut args = env::args().skip(1);
    let path = args.next().expect("usage: replay_rel <trace.log> [key=value ...]");
    let mut cfg = Config::FIRMWARE_DEFAULT;
    let (mut scroll, mut quiet, mut skip_fc) = (false, false, false);
    for kv in args {
        let (k, v) = kv.split_once('=').expect("key=value");
        let n: i64 = v.parse().expect("number");
        match k {
            "gain" => cfg.gain = n as i32,
            "gain_div" => cfg.gain_div = n as i32,
            "swap" => cfg.swap_xy = n != 0,
            "invx" => cfg.invert_x = n != 0,
            "invy" => cfg.invert_y = n != 0,
            "scroll_div" => cfg.scroll_div = n as i32,
            "scroll_invert" => cfg.scroll_invert = n != 0,
            "still" => cfg.still = (n != 0).then_some(StillFilter::DEFAULT),
            "tap" => cfg.soft_tap = (n != 0).then_some(SoftTap::DEFAULT),
            "scroll" => scroll = n != 0,
            "quiet" => quiet = n != 0,
            "skip_fc" => skip_fc = n != 0,
            _ => panic!("unknown key {k}"),
        }
    }
    let text = fs::read_to_string(&path).expect("read trace");
    let mut relay = Relay::new(cfg);
    let (mut packets, mut zero_packets, mut reports, mut clicks) = (0usize, 0usize, 0usize, 0usize);
    let (mut sum_x, mut sum_y, mut max_step) = (0i64, 0i64, 0i32);
    let mut last_buttons = 0u8;
    let mut boots = 0usize;
    let mut last_ms: Option<u32> = None;
    let mut pending: Vec<[u8; 5]> = Vec::new();
    for line in text.lines() {
        if line.contains("ESP-ROM") {
            boots += 1;
            if !quiet {
                println!("---- boot {boots}");
            }
        }
        let Some(l) = parse_trace_line(line) else { continue };
        if skip_fc && l.raw == [0xfc, 0xfc, 0xfc] {
            continue;
        }
        // ファームと同じく、パケットの間は 5ms ごとに poll する（時間で決まる処理のため）
        if let Some(prev) = last_ms {
            if l.ms > prev && l.ms - prev < 5000 {
                let mut t = prev + 5;
                while t < l.ms {
                    relay.poll(t, |r| pending.push(r));
                    t += 5;
                }
            }
        }
        last_ms = Some(l.ms);
        let p = l.packet();
        packets += 1;
        if p.dx == 0 && p.dy == 0 && p.buttons == 0 {
            zero_packets += 1;
        }
        let mut out = std::mem::take(&mut pending);
        relay.push(l.ms, &p, scroll, |r| out.push(r));
        for r in &out {
            reports += 1;
            let (x, y) = (r[1] as i8 as i32, r[2] as i8 as i32);
            sum_x += x as i64;
            sum_y += y as i64;
            max_step = max_step.max(x.abs()).max(y.abs());
            if r[0] & 1 != 0 && last_buttons & 1 == 0 {
                clicks += 1;
            }
            last_buttons = r[0];
        }
        if !quiet {
            let rendered: Vec<String> = out
                .iter()
                .map(|r| format!("btn={} x={} y={} wheel={}", r[0], r[1] as i8, r[2] as i8, r[3] as i8))
                .collect();
            println!("{:>8} raw={:02x?} dx={:>4} dy={:>4} btn={}  -> {}", l.ms, l.raw, p.dx, p.dy, p.buttons, rendered.join(" | "));
        }
    }
    println!("--");
    println!("boots={boots} packets={packets} zero_packets={zero_packets} reports={reports} clicks={clicks}");
    println!("sum x={sum_x} y={sum_y}  max single step={max_step}");
}

#!/usr/bin/env python3
"""ショートカットリングの確認用クライアント（PC の BLE で hikyaku の独自サービス ring を受け取って表示する）。

使い方: tools/ring/ring.sh [--addr FF:E4:05:1A:8F:FF]
  o: リングを開く（F23 と同じ）  c: 閉じる  r: 生の値の最小・最大をリセット  q: 終了

hikyaku は 1 台としか接続しないので、Mac の Bluetooth を切ってから使う（hikyaku が広告を始める）。
イベントの形式は crates/hikyaku-pointer/src/ring.rs、仕様は docs/design/ring.md。
"""
import argparse
import asyncio
import math
import os
import struct
import sys
import termios
import time
import tty

from bleak import BleakClient, BleakScanner

STATE = "6b9a0011-6f1e-4b8e-9d57-7f0d0a5b0001"
CTRL = "6b9a0012-6f1e-4b8e-9d57-7f0d0a5b0001"
HELLO, CLOSE, OPEN = 0x00, 0x01, 0x02
SECTORS = 8
CANCEL_RADIUS = 350  # これより中心寄りで離したらキャンセル（アプリ側で持つ値の仮）


class View:
    def __init__(self):
        self.open = False
        self.touching = False
        self.angle = 0.0
        self.radius = 0
        self.raw = (0, 0, 0)
        self.minmax = None
        self.count = 0
        self.lost = 0
        self.last_seq = None
        self.t0 = time.time()
        self.message = ""
        self.last_sector = None

    def on_event(self, b: bytes):
        kind, flags, angle, radius, x, y, z, seq = struct.unpack("<BBHHHHBB", b[:12])
        if self.last_seq is not None and (seq - self.last_seq) % 256 != 1:
            self.lost += (seq - self.last_seq - 1) % 256
        self.last_seq = seq
        if kind == 1:
            self.open = True
            self.message = "開いた"
        elif kind == 2:
            self.open = False
            self.touching = False
            self.message = "閉じた"
        elif kind == 3:
            self.count += 1
            was = self.touching
            self.touching = bool(flags & 1)
            if self.touching:
                self.angle, self.radius = angle / 100.0, radius
                self.raw = (x, y, z)
                if self.minmax is None:
                    self.minmax = [x, x, y, y]
                else:
                    m = self.minmax
                    m[0], m[1], m[2], m[3] = min(m[0], x), max(m[1], x), min(m[2], y), max(m[3], y)
                self.last_sector = self.sector()
            elif was:
                # 離した: アプリならここで実行かキャンセル
                if self.last_sector is None:
                    self.message = "離した → キャンセル（中心付近）"
                else:
                    self.message = f"離した → 項目 {self.last_sector}（{self.last_sector * 45}°）を実行"
        self.draw()

    def sector(self):
        if self.radius < CANCEL_RADIUS:
            return None
        return int(((self.angle + 22.5) % 360) // 45)

    def draw(self):
        w, h = 41, 17  # 文字のセルは縦長なので、横を 2 倍にして円にする
        cx, cy, r = w // 2, h // 2, 7
        grid = [[" "] * w for _ in range(h)]
        for i in range(64):
            t = 2 * math.pi * i / 64
            grid[round(cy - r * math.cos(t))][round(cx + 2 * r * math.sin(t))] = "·"
        sel = self.sector() if self.touching else None
        for s in range(SECTORS):
            t = math.radians(s * 45)
            gx, gy = round(cx + 2 * (r + 1.5) * math.sin(t)), round(cy - (r + 1.5) * math.cos(t))
            grid[gy][gx] = "●" if s == sel else str(s)
        if self.touching:
            t = math.radians(self.angle)
            rr = min(self.radius, 1200) / 1000 * r
            gy = min(max(round(cy - rr * math.cos(t)), 0), h - 1)
            gx = min(max(round(cx + 2 * rr * math.sin(t)), 0), w - 1)
            grid[gy][gx] = "+"
        grid[cy][cx] = "x" if not self.touching else grid[cy][cx]
        rate = self.count / max(time.time() - self.t0, 1e-3)
        out = ["\x1b[H\x1b[2J", "hikyaku ring  [o]開く [c]閉じる [r]範囲リセット [q]終了\r\n\r\n"]
        out += ["  " + "".join(row) + "\r\n" for row in grid]
        out.append(f"\r\n  状態: {'開' if self.open else '閉'}  触れている: {'はい' if self.touching else 'いいえ'}\r\n")
        out.append(f"  角度: {self.angle:6.1f}°  距離: {self.radius:4d}/1000  項目: {sel if sel is not None else '-'}\r\n")
        x, y, z = self.raw
        out.append(f"  生の値: x={x:4d} y={y:4d} z={z:2d}   受信 {self.count}（{rate:.0f}/秒）  取りこぼし {self.lost}\r\n")
        if self.minmax:
            x0, x1, y0, y1 = self.minmax
            out.append(f"  x の範囲 {x0}〜{x1}  y の範囲 {y0}〜{y1}  → 中心 ({(x0 + x1) // 2}, {(y0 + y1) // 2})"
                       f"  半径 ({(x1 - x0) // 2}, {(y1 - y0) // 2})（縁をなぞって測る）\r\n")
        out.append(f"\r\n  {self.message}\r\n")
        sys.stdout.write("".join(out))
        sys.stdout.flush()


async def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--addr", default="FF:E4:05:1A:8F:FF")
    args = ap.parse_args()

    print(f"{args.addr} を探しています…（Mac の Bluetooth は切っておく）")
    dev = await BleakScanner.find_device_by_address(args.addr, timeout=20)
    if dev is None:
        print("見つからない。hikyaku が Mac とつながったままなら、Mac の Bluetooth を切る。")
        return 1
    view = View()
    async with BleakClient(dev) as client:
        await client.start_notify(STATE, lambda _, d: view.on_event(bytes(d)))
        await client.write_gatt_char(CTRL, bytes([HELLO]), response=True)
        view.message = "接続した（HELLO を送った）。o で開く、または F23"
        view.draw()

        fd = sys.stdin.fileno()
        old = termios.tcgetattr(fd)
        tty.setcbreak(fd)
        loop = asyncio.get_running_loop()
        keys: asyncio.Queue[str] = asyncio.Queue()
        loop.add_reader(fd, lambda: keys.put_nowait(os.read(fd, 1).decode(errors="ignore")))
        try:
            while client.is_connected:
                try:
                    k = await asyncio.wait_for(keys.get(), timeout=0.5)
                except asyncio.TimeoutError:
                    continue
                if k == "q":
                    break
                if k == "o":
                    await client.write_gatt_char(CTRL, bytes([OPEN]), response=True)
                elif k == "c":
                    await client.write_gatt_char(CTRL, bytes([CLOSE]), response=True)
                elif k == "r":
                    view.minmax = None
                    view.draw()
        finally:
            loop.remove_reader(fd)
            termios.tcsetattr(fd, termios.TCSADRAIN, old)
            try:
                await client.write_gatt_char(CTRL, bytes([CLOSE]), response=True)
            except Exception:
                pass
    print("\r\n切断した")
    return 0


if __name__ == "__main__":
    sys.exit(asyncio.run(main()))

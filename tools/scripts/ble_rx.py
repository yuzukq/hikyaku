#!/usr/bin/env python3
"""この PC を BLE の受信側にして、hikyaku から届く HID レポートを表示する。

使い方: ble_rx.py <秒数>

bluetoothctl でペアリング・接続し、hikyaku の hidraw ノードを読んで 16 進表示する。
終了時にペアリングを削除する。/dev/hidraw* を読める権限が必要（docs/firmware.md の udev ルール）。
レポートの先頭 1 バイトがレポート ID（01=キーボード 8B, 02=マウス 5B, 03=コンシューマ 2B）。
"""
import glob
import os
import subprocess
import sys
import threading
import time

ADDR = "FF:E4:05:1A:8F:FF"  # firmware/hikyaku-s3 で固定しているランダムアドレス
T = float(sys.argv[1])
t0 = time.time()


def log(m):
    print("%5.1f %s" % (time.time() - t0, m), flush=True)


def find():
    for p in sorted(glob.glob("/sys/class/hidraw/hidraw*")):
        try:
            if "HID_NAME=hikyaku" in open(p + "/device/uevent").read():
                return "/dev/" + p.split("/")[-1]
        except OSError:
            pass


def reader():
    while True:
        dev = find()
        if not dev:
            time.sleep(0.1)
            continue
        log("found %s readable=%s" % (dev, os.access(dev, os.R_OK)))
        try:
            f = open(dev, "rb", buffering=0)  # バッファ付きだと read(64) が 64 バイト溜まるまで返らない
            while True:
                log("RECEIVED %s: %s" % (dev, f.read(64).hex()))
        except OSError as e:
            log("reader: %s" % e)
            time.sleep(1)


threading.Thread(target=reader, daemon=True).start()
bt = subprocess.Popen(["bluetoothctl"], stdin=subprocess.PIPE, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, text=True)


def w(c, d=0):
    bt.stdin.write(c + "\n")
    bt.stdin.flush()
    time.sleep(d)


w("remove " + ADDR)
w("scan on", 6)
w("pair " + ADDR, 6)
w("trust " + ADDR)
w("connect " + ADDR)
log("connected; type on the keyboard now")
time.sleep(T)
w("disconnect " + ADDR, 1)
w("remove " + ADDR)
w("quit", 1)

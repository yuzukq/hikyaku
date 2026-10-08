#!/usr/bin/env python3
"""シリアルポートのログを一定時間キャプチャして標準出力へ出す。

使い方: uart_cap.py <秒数> [ポート=/dev/ttyACM0]

DTR/RTS を落とした状態で開く（USB-Serial-JTAG の ESP はポートを開くたびに
リセットされることがあるため、注意。docs/firmware.md 参照）。
"""
import sys
import time

import serial

dur = float(sys.argv[1])
port = sys.argv[2] if len(sys.argv) > 2 else "/dev/ttyACM0"
s = serial.Serial(port, 115200, timeout=0.5, dsrdtr=False, rtscts=False)
s.dtr = False
s.rts = False
t = time.time()
while time.time() - t < dur:
    d = s.read(4096)
    if d:
        sys.stdout.write("%5.1f " % (time.time() - t) + d.decode(errors="replace").replace("\x1b", ""))
        sys.stdout.flush()

#!/usr/bin/env python3
"""シリアルポートのログを一定時間キャプチャして標準出力へ出す。

使い方: uart_cap.py <秒数> [ポート=/dev/ttyACM0]

DTR/RTS を落とした状態で開く。ポートが一時的に消えたり、読み取りエラーになったりしたら、開き直して続ける。
"""
import sys
import time

import serial

dur = float(sys.argv[1])
port = sys.argv[2] if len(sys.argv) > 2 else "/dev/ttyACM0"
t = time.time()
s = None
while time.time() - t < dur:
    try:
        if s is None:
            s = serial.Serial(port, 115200, timeout=0.5, dsrdtr=False, rtscts=False)
            s.dtr = False
            s.rts = False
        d = s.read(4096)
        if d:
            sys.stdout.write("%5.1f " % (time.time() - t) + d.decode(errors="replace").replace("\x1b", ""))
            sys.stdout.flush()
    except (serial.SerialException, OSError) as e:
        sys.stdout.write("%5.1f [uart_cap] %s; reopening\n" % (time.time() - t, e))
        sys.stdout.flush()
        try:
            if s is not None:
                s.close()
        except Exception:
            pass
        s = None
        time.sleep(0.5)

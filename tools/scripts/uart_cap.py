#!/usr/bin/env python3
"""シリアルポートのログを一定時間キャプチャして標準出力へ出す。

使い方: uart_cap.py <秒数> [ポート=/dev/ttyACM0]

DTR/RTS を落とした状態で開く。ポートが一時的に消えたり、読み取りエラーになったりしたら、開き直して続ける。\nCH9102F（ベンダー ID 1a86）以外のポートは開かない（同じ /dev/ttyACM0 に S3 が現れても、書き込みと取り合わないように）。
"""
import subprocess
import sys
import time

import serial


def is_ch9102(port: str) -> bool:
    """そのポートが CH9102F（ベンダー ID 1a86）か。S3 の書き込みポートと取り合わないため、それ以外は開かない。"""
    try:
        out = subprocess.run(["udevadm", "info", "-q", "property", "-n", port], capture_output=True, text=True).stdout
    except OSError:
        return False
    return "ID_VENDOR_ID=1a86" in out

dur = float(sys.argv[1])
port = sys.argv[2] if len(sys.argv) > 2 else "/dev/ttyACM0"
t = time.time()
s = None
while time.time() - t < dur:
    try:
        if s is None:
            if not is_ch9102(port):
                time.sleep(0.5)
                continue
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

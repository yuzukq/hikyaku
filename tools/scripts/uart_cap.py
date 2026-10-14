#!/usr/bin/env python3
"""シリアルポートのログを一定時間キャプチャして標準出力へ出す。

使い方: uart_cap.py <秒数> [ポート=/dev/ttyACM0]

DTR/RTS を落とした状態で開く。ポートが一時的に消えたり、読み取りエラーになったりしたら、開き直して続ける。\nCH9102F（ベンダー ID 1a86）以外のポートは開かない（同じ /dev/ttyACM0 に S3 が現れても、書き込みと取り合わないように）。
環境変数 UART_CAP_VENDOR で開くベンダー ID を変えられる（S3 の USB シリアル/JTAG から読むときは 303a）。
"""
import os
import subprocess
import sys
import time

import serial


VENDOR = os.environ.get("UART_CAP_VENDOR", "1a86")


def is_ch9102(port: str) -> bool:
    """そのポートが CH9102F（ベンダー ID 1a86。UART_CAP_VENDOR で変更可）か。S3 の書き込みポートと取り合わないため、それ以外は開かない。"""
    try:
        out = subprocess.run(["udevadm", "info", "-q", "property", "-n", port], capture_output=True, text=True).stdout
    except OSError:
        return False
    return "ID_VENDOR_ID=" + VENDOR in out

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
            # 開く前に DTR/RTS を決めておく（S3 の USB シリアル/JTAG では、開くときに切り替わるとリセットされる）
            s = serial.Serial(None, 115200, timeout=0.5, dsrdtr=False, rtscts=False)
            s.port = port
            s.dtr = False
            s.rts = False
            s.open()
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

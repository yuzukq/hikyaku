#!/usr/bin/env python3
"""起動中の Fusion に、MCP サーバー経由で hikyaku4pcb_case.py の build() を実行させる。

Fusion の「スクリプトとアドイン」から実行するのと同じものができる。違いは 2 つ:
  - 開いているドキュメントの中から、名前に hikyaku4pcb が入っているものを探して、それをアクティブにしてから作る
  - 結果のメッセージボックスを出さず、警告・壊れたフィーチャ・各ボディの体積（mm3）を標準出力に出す

    python3 run_in_fusion.py                # 作り直す
    python3 run_in_fusion.py --shot out.png # 作り直して、斜め上からのスクリーンショットを保存

前提: Fusion が起動していて、hikyaku4pcb を開いている。基本設定の「一般」→「Fusion MCP サーバ」が有効。
保存はしない（Fusion で手で保存する）。
"""
import argparse
import base64
import json
import os
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
CASE = os.path.join(HERE, "hikyaku4pcb_case", "hikyaku4pcb_case.py")
DOC = "hikyaku4pcb"

RUN = '''

def run(_context: str):
    import time
    app = adsk.core.Application.get()
    docs = [d for d in app.documents if "%(doc)s" in d.name]
    if len(docs) != 1:
        raise RuntimeError("%(doc)s を開いてください（開いているもの: %%s）" %% [d.name for d in app.documents])
    docs[0].activate()
    design = adsk.fusion.Design.cast(docs[0].products.itemByProductType("DesignProductType"))
    up = UP if UP in ("Y", "Z") else detect_up(app)
    K = FusionBackend(design, up)
    t = time.time()
    build(K)
    print("作成しました（%%s 上向き、%%.1f 秒）" %% (up, time.time() - t))
    for w in K.warnings:
        print("注意:", w)
    tl = design.timeline
    for i in range(tl.count):
        it = tl.item(i)
        if it.healthState != adsk.fusion.FeatureHealthStates.HealthyFeatureHealthState:
            print("壊れたフィーチャ:", it.name, it.errorOrWarningMessage)
    for o in design.rootComponent.occurrences:
        c = o.component
        print(c.name, ", ".join("%%s %%.1f" %% (b.name, b.volume * 1000) for b in c.bRepBodies))
    app.activeViewport.fit()
''' % {"doc": DOC}


class FusionMCP:
    def __init__(self, url):
        self.url = url
        self.sid = None
        self.sid = self._post({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
            "protocolVersion": "2025-06-18", "capabilities": {},
            "clientInfo": {"name": "hikyaku4pcb", "version": "1"}}})[0]
        self._post({"jsonrpc": "2.0", "method": "notifications/initialized"})

    def _post(self, body):
        h = {"Content-Type": "application/json", "Accept": "application/json, text/event-stream"}
        if self.sid:
            h["MCP-Session-Id"] = self.sid
        req = urllib.request.Request(self.url, json.dumps(body).encode(), h)
        with urllib.request.urlopen(req, timeout=600) as r:
            txt = r.read().decode()
            return r.headers.get("MCP-Session-Id"), (json.loads(txt) if txt.strip() else None)

    def call(self, tool, args):
        _, r = self._post({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
                           "params": {"name": tool, "arguments": args}})
        if "error" in r:
            raise SystemExit("MCP のエラー: %s" % r["error"])
        return r["result"].get("content", [])


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--url", default="http://127.0.0.1:27182/mcp",
                    help="Fusion の MCP サーバー（ポートが使われていると別の番号になる。lsof -iTCP -sTCP:LISTEN で確かめる）")
    ap.add_argument("--shot", help="作ったあとのスクリーンショットの保存先（PNG）")
    a = ap.parse_args()

    m = FusionMCP(a.url)
    script = open(CASE, encoding="utf-8").read() + RUN
    failed = False
    for c in m.call("fusion_mcp_execute", {"featureType": "script", "object": {"script": script}}):
        if c.get("type") != "text":
            continue
        try:
            res = json.loads(c["text"])
        except ValueError:
            print(c["text"])
            continue
        failed = not res.get("success")
        print(res.get("message") or res.get("error") or res)
    if failed:
        raise SystemExit(1)
    if a.shot:
        for c in m.call("fusion_mcp_read", {"queryType": "screenshot", "direction": "iso-top-right",
                                            "width": 1200, "height": 900, "transparentBackground": False}):
            if c.get("type") == "image":
                with open(a.shot, "wb") as f:
                    f.write(base64.b64decode(c["data"]))
                print("保存しました:", a.shot)


if __name__ == "__main__":
    main()

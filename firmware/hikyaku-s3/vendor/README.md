# vendor/

hikyaku のために手を入れた依存クレート。`Cargo.toml` の `[patch.crates-io]` で、crates.io の版の代わりに使う。

## trouble-host（0.7.0 のコピー。ライセンスは Apache-2.0 OR MIT）

変更（`hikyaku:` のコメントのところ）:

- **相手からの 2 回目以降の ATT MTU 交換の要求: 暗号化済みならすぐに MTU 応答、暗号化の前なら静かなときにエラー応答（Request Not Supported）**（`src/host.rs`、
  `src/connection_manager.rs` の `first_att_mtu_request`・`defer_mtu_response`・`send_deferred_mtu_response`、
  `src/connection.rs` の `Connection::send_deferred_mtu_response`。アプリが 100 ms ごとに呼ぶ: main.rs の `session`）。
  Windows（TP-Link / Realtek の USB アダプタ）は、同じ Exchange MTU Request を同じ瞬間に 2 回送る（Windows の中の 2 つの部品が
  それぞれ出しているらしい）。元の trouble-host のように 2 回ともすぐ応答すると、2 回目の応答を別の要求への応答と取り違えて切断し、
  応答しないと 2 回目の要求が 30 秒の時間切れになって切断した（どちらも Windows の ETW で確認、2026-10-24）。
  静かなときに MTU の応答を返しても、Windows は MTU が決まった後の MTU の応答を捨てるので、やはり 30 秒で切れた。
  そこで 2 回目の要求には、相手から ATT が 500 ms 来ていないとき（遅くとも 20 秒後）に、エラー応答
  `[01 02 00 00 06]`（Request Not Supported）を返して、その要求を終わらせる。
  ところが遅らせた応答は、MTU 応答でもエラー応答でも Windows に捨てられ、やはり 30 秒で切れた。つなぎ直し（暗号化の後）では、
  Windows は 1 回目の応答の直後に次の要求を出していないので、2 回目にもすぐ応答する（`is_encrypted_handle`）。遅らせるのは、
  ペアリング中（暗号化の前）だけにする。
- **周辺機器として接続したら、こちらから ATT MTU 交換の要求を送る**（`src/host.rs` の `send_own_mtu_request`。応答は GATT
  クライアントに渡さない: `own_mtu_request_pending`）。BlueZ や市販の機器と同じ動き。Windows の二重の要求は止まらなかったが、害はないので残す。

trouble-host を新しい版に上げるときは、この変更が取り込まれているか確かめ、要らなければ `vendor/trouble-host` と `[patch.crates-io]` を消す。

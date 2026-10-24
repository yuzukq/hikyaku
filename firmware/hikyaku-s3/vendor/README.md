# vendor/

hikyaku のために手を入れた依存クレート。`Cargo.toml` の `[patch.crates-io]` で、crates.io の版の代わりに使う。

## trouble-host（0.7.0 のコピー。ライセンスは Apache-2.0 OR MIT）

変更は 1 つだけ（`hikyaku:` のコメントのところ）:

- **ATT の MTU 交換の要求に、1 接続で 1 回だけ応答する**（`src/connection_manager.rs` の `first_att_mtu_request`、`src/host.rs`）。
  Windows（TP-Link / Realtek の USB アダプタ）が同じ Exchange MTU Request を同じ瞬間に 2 回送り、元の trouble-host は
  2 回とも応答していた。Windows は 2 回目の応答を「頼んでいない応答」とみなして、ペアリングの途中でも切断していた
  （2026-10-24、Windows の ETW で確認。docs/design/multi-host.md）。規格でも、クライアントは MTU 交換を 1 接続に 1 回しか送らない。

trouble-host を新しい版に上げるときは、この変更が取り込まれているか確かめ、要らなければ `vendor/trouble-host` と `[patch.crates-io]` を消す。

//! トラックパッドの入力を、マウスレポートに変換する。
//!
//! ハードウェアに依存しない純粋なロジック。`no_std`・時刻は呼び出し側が与える。
//! Pinnacle の相対モードのパケットを扱う [`relative`]、ショートカットリングの [`ring`]、本体のボタンの判定 [`button`]。
//! 絶対座標から移動量を作る旧方式（`Pointer`・`tracker`）は 2026-10-13 に削除した（タグ `absolute-mode-last` に残っている）。

#![cfg_attr(not(test), no_std)]

pub mod button;
pub mod relative;
pub mod ring;

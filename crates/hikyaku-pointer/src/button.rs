//! hikyaku 本体のボタン（MX スイッチ 1 個）: 単押し = 右クリック、長押し中 = スクロール、ダブルタップ = リングの開閉。
//!
//! 見分け方:
//! 1. 押した瞬間にスクロールを始める（長押しを待たないので、押してすぐパッドを動かせばスクロールになる）。
//! 2. 押している間にパッドが動いたら長押し。離せばスクロールが終わるだけで、クリックは出さない。
//! 3. 動かさずに短く（`tap_max_ms` 以内に）離したら単押しの候補。`double_gap_ms` 以内にもう一度押したらダブルタップ
//!    （2 回目を押した瞬間にリング）。来なければ右クリック。**右クリックはその分（`double_gap_ms`）遅れて出る。**
//!
//! 時刻は呼び出し側が与える（ミリ秒、u32 のラップアラウンド可）。チャタリングは、状態が変わってから `debounce_ms` の間の
//! 変化を無視して吸収する（押した瞬間はすぐ反応する）。

/// 判定の時間（ミリ秒）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timing {
    /// 状態が変わってから、この間の変化はチャタリングとして無視する
    pub debounce_ms: u32,
    /// これより長く押していたら単押しにしない
    pub tap_max_ms: u32,
    /// 単押しを離してから、この間にもう一度押したらダブルタップ
    pub double_gap_ms: u32,
}

impl Timing {
    pub const DEFAULT: Timing = Timing { debounce_ms: 5, tap_max_ms: 200, double_gap_ms: 250 };
}

/// ボタンの判定の結果、ファームがすること。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// パッドの動きをスクロールにし始める
    ScrollStart,
    /// スクロールをやめる
    ScrollEnd,
    /// 右クリック（押して離す）
    RightClick,
    /// ショートカットリングを開く / 閉じる（F23 と同じ）
    RingToggle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Idle,
    /// 1 回目を押している（moved = その間にパッドが動いた）
    Held { since: u32, moved: bool },
    /// 単押しを離した。2 回目を待っている
    WaitSecond { released: u32 },
    /// ダブルタップの 2 回目を押している（離すまで何もしない）
    SecondHeld,
}

pub struct Button {
    timing: Timing,
    state: State,
    /// チャタリングを除いた、押しているか
    pressed: bool,
    /// 最後に状態が変わった時刻
    changed: Option<u32>,
}

impl Button {
    pub const fn new(timing: Timing) -> Self {
        Self { timing, state: State::Idle, pressed: false, changed: None }
    }

    /// スクロール中（1 回目を押している間）か。
    pub fn scrolling(&self) -> bool {
        matches!(self.state, State::Held { .. })
    }

    /// 押している間にパッドが動いた（スクロールした）。このあと離しても右クリックにしない。
    pub fn moved(&mut self) {
        if let State::Held { moved, .. } = &mut self.state {
            *moved = true;
        }
    }

    /// ピンの状態（押している = true）を渡す。5ms ごとくらいに呼ぶ。
    pub fn update(&mut self, now: u32, pressed: bool) -> Option<Action> {
        // 2 回目が来ないまま時間切れ → 右クリック
        if let State::WaitSecond { released } = self.state {
            if !pressed && now.wrapping_sub(released) > self.timing.double_gap_ms {
                self.state = State::Idle;
                return Some(Action::RightClick);
            }
        }
        if pressed == self.pressed || self.changed.is_some_and(|t| now.wrapping_sub(t) < self.timing.debounce_ms) {
            return None;
        }
        self.pressed = pressed;
        self.changed = Some(now);
        match (self.state, pressed) {
            (State::Idle, true) => {
                self.state = State::Held { since: now, moved: false };
                Some(Action::ScrollStart)
            }
            (State::WaitSecond { .. }, true) => {
                self.state = State::SecondHeld;
                Some(Action::RingToggle)
            }
            (State::Held { since, moved }, false) => {
                let tap = !moved && now.wrapping_sub(since) <= self.timing.tap_max_ms;
                self.state = if tap { State::WaitSecond { released: now } } else { State::Idle };
                Some(Action::ScrollEnd)
            }
            (State::SecondHeld, false) => {
                self.state = State::Idle;
                None
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ピンの状態の列（開始時刻, 押している）を 1ms ごとに流し、出た結果を（時刻, 結果）で返す。
    fn run(b: &mut Button, steps: &[(u32, bool)], until: u32) -> Vec<(u32, Action)> {
        let mut out = Vec::new();
        let mut level = false;
        for t in 0..until {
            if let Some(&(_, p)) = steps.iter().rev().find(|&&(s, _)| s <= t) {
                level = p;
            }
            if let Some(a) = b.update(t, level) {
                out.push((t, a));
            }
        }
        out
    }

    fn actions(v: &[(u32, Action)]) -> Vec<Action> {
        v.iter().map(|&(_, a)| a).collect()
    }

    #[test]
    fn single_press_is_a_right_click_after_the_double_tap_window() {
        let mut b = Button::new(Timing::DEFAULT);
        let out = run(&mut b, &[(10, true), (100, false)], 600);
        assert_eq!(actions(&out), [Action::ScrollStart, Action::ScrollEnd, Action::RightClick]);
        // 離してから double_gap_ms 過ぎたところで出る
        assert_eq!(out[2].0, 100 + 251);
    }

    #[test]
    fn hold_and_move_scrolls_without_a_click() {
        let mut b = Button::new(Timing::DEFAULT);
        let mut out = run(&mut b, &[(10, true)], 50);
        assert!(b.scrolling());
        b.moved();
        // 短く離しても、動かしていたらクリックにしない
        out.extend((50..600).filter_map(|t| b.update(t, t < 120).map(|a| (t, a))));
        assert_eq!(actions(&out), [Action::ScrollStart, Action::ScrollEnd]);
        assert!(!b.scrolling());
    }

    #[test]
    fn long_press_without_moving_does_nothing() {
        let mut b = Button::new(Timing::DEFAULT);
        let out = run(&mut b, &[(10, true), (400, false)], 1000);
        assert_eq!(actions(&out), [Action::ScrollStart, Action::ScrollEnd]);
    }

    #[test]
    fn double_tap_toggles_the_ring_on_the_second_press() {
        let mut b = Button::new(Timing::DEFAULT);
        let out = run(&mut b, &[(10, true), (80, false), (200, true), (260, false)], 1000);
        assert_eq!(actions(&out), [Action::ScrollStart, Action::ScrollEnd, Action::RingToggle]);
        assert_eq!(out[2].0, 200);
        assert!(!b.scrolling(), "the second press does not scroll");
    }

    #[test]
    fn bounces_are_ignored() {
        let mut b = Button::new(Timing::DEFAULT);
        // 押した直後と離した直後に 1ms ずつ跳ねる
        let steps = [(10, true), (11, false), (12, true), (100, false), (102, true), (103, false)];
        let out = run(&mut b, &steps, 600);
        assert_eq!(actions(&out), [Action::ScrollStart, Action::ScrollEnd, Action::RightClick]);
    }

    #[test]
    fn works_across_the_clock_wrapping() {
        let mut b = Button::new(Timing::DEFAULT);
        let t0 = u32::MAX - 50;
        let mut out = Vec::new();
        for i in 0..600u32 {
            let t = t0.wrapping_add(i);
            if let Some(a) = b.update(t, (10..100).contains(&i)) {
                out.push(a);
            }
        }
        assert_eq!(out, [Action::ScrollStart, Action::ScrollEnd, Action::RightClick]);
    }
}

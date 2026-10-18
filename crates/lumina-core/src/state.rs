//! Island states, normalized input events, and the priority based transition
//! arbiter.
//!
//! # The arbitration problem
//!
//! Several things can want the island at once. A track starts playing while the
//! pointer is hovering and a notification is arriving. Something has to decide
//! what the island shows, and it has to decide the same way every time or the
//! behavior feels random.
//!
//! The rule is priority with a hold window:
//!
//! 1. Each widget that wants the island publishes a [`StateRequest`] with a
//!    priority, an optional expiry, and a pinned flag.
//! 2. The arbiter picks the highest priority among live requests.
//! 3. A request stays live until it expires, until the pointer is inside and it
//!    is the hover candidate, or until it is explicitly cleared.
//!
//! Because the arbiter is a pure function of the request set, it is exhaustively
//! testable without any timing or windowing code.

use serde::{Deserialize, Serialize};

use crate::geometry::Geometry;

/// The visual states the island can be in.
///
/// Geometry defaults are the logical pixel values at 100 percent DPI. The
/// renderer scales them by the monitor scale factor before uploading uniforms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum IslandState {
    /// At rest. The default and the lowest priority.
    #[default]
    Collapsed,
    /// Pointer is inside the island and nothing more important is live.
    Hover,
    /// Media transport and metadata.
    Media,
    /// Live audio spectrum.
    Spectrum,
    /// Clock, date, calendar.
    Clock,
    /// System metrics.
    Metrics,
    /// User configured actions.
    Shortcuts,
    /// A transient message.
    Notify,
}

/// The states a user can cycle through, in presentation order.
///
/// Deliberately not the enum's declaration order. Presentation order is what
/// the cycle hotkey follows, and it is an independent decision from which
/// variant the compiler happens to lay out first.
pub const CYCLE_ORDER: [IslandState; 6] = [
    IslandState::Clock,
    IslandState::Media,
    IslandState::Spectrum,
    IslandState::Metrics,
    IslandState::Shortcuts,
    IslandState::Notify,
];

impl IslandState {
    /// All states, in declaration order.
    pub const ALL: [Self; 8] = [
        Self::Collapsed,
        Self::Hover,
        Self::Media,
        Self::Spectrum,
        Self::Clock,
        Self::Metrics,
        Self::Shortcuts,
        Self::Notify,
    ];

    /// Stable identifier used in configuration and IPC.
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::Collapsed => "collapsed",
            Self::Hover => "hover",
            Self::Media => "media",
            Self::Spectrum => "spectrum",
            Self::Clock => "clock",
            Self::Metrics => "metrics",
            Self::Shortcuts => "shortcuts",
            Self::Notify => "notify",
        }
    }

    /// Parse an identifier, case insensitive and tolerant of separators.
    ///
    /// Used by IPC and config so `spectrum`, `Spectrum`, and `SPECTRUM` all
    /// resolve to the same state.
    #[must_use]
    pub fn from_id(input: &str) -> Option<Self> {
        let normalized: String = input
            .trim()
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .map(|c| c.to_ascii_lowercase())
            .collect();
        Self::ALL.into_iter().find(|s| s.id() == normalized)
    }

    /// Arbitration priority. Higher wins.
    ///
    /// Ordering rationale: a notification the user explicitly asked for should
    /// not be hidden by a spectrum. Media outranks spectrum because the
    /// metadata is the information and the spectrum is the decoration. Hover
    /// outranks nothing but collapse, so a passive pointer never blocks
    /// anything.
    #[must_use]
    pub const fn priority(self) -> u8 {
        match self {
            Self::Collapsed => 0,
            Self::Hover => 1,
            Self::Clock => 2,
            Self::Metrics => 3,
            Self::Shortcuts => 4,
            Self::Media => 5,
            Self::Spectrum => 6,
            Self::Notify => 7,
        }
    }

    /// Content geometry for this state.
    ///
    /// These numbers are the shipped defaults. Every one of them is
    /// user-overridable through the config schema, which is why they live in one
    /// table rather than scattered through the renderer.
    #[must_use]
    pub const fn geometry(self) -> Geometry {
        match self {
            // half_top, half_bottom, height, radius
            Self::Collapsed => Geometry {
                half_top: 62.0,
                half_bottom: 54.0,
                height: 18.0,
                radius: 9.0,
            },
            Self::Hover => Geometry {
                half_top: 66.0,
                half_bottom: 56.0,
                height: 22.0,
                radius: 11.0,
            },
            Self::Media => Geometry {
                half_top: 180.0,
                half_bottom: 168.0,
                height: 40.0,
                radius: 20.0,
            },
            Self::Spectrum => Geometry {
                half_top: 210.0,
                half_bottom: 200.0,
                height: 50.0,
                radius: 24.0,
            },
            Self::Clock => Geometry {
                half_top: 100.0,
                half_bottom: 92.0,
                height: 30.0,
                radius: 15.0,
            },
            Self::Metrics => Geometry {
                half_top: 120.0,
                half_bottom: 112.0,
                height: 28.0,
                radius: 14.0,
            },
            Self::Shortcuts => Geometry {
                half_top: 170.0,
                half_bottom: 160.0,
                height: 34.0,
                radius: 17.0,
            },
            Self::Notify => Geometry {
                half_top: 200.0,
                half_bottom: 190.0,
                height: 28.0,
                radius: 14.0,
            },
        }
    }

    /// Content opacity in this state, used by the content fade spring.
    ///
    /// `Collapsed` shows the shape alone, so its content is fully transparent.
    /// `Hover` shows dimmed icons, so it is partially visible. Everything else
    /// is a full content state.
    #[must_use]
    pub const fn content_opacity(self) -> f32 {
        match self {
            Self::Collapsed => 0.0,
            Self::Hover => 0.55,
            Self::Media
            | Self::Spectrum
            | Self::Clock
            | Self::Metrics
            | Self::Shortcuts
            | Self::Notify => 1.0,
        }
    }

    /// True when this state shows no content, only the shape.
    #[must_use]
    pub const fn is_contentless(self) -> bool {
        matches!(self, Self::Collapsed)
    }

    /// Index in [`IslandState::ALL`], used for cycling.
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::Collapsed => 0,
            Self::Hover => 1,
            Self::Media => 2,
            Self::Spectrum => 3,
            Self::Clock => 4,
            Self::Metrics => 5,
            Self::Shortcuts => 6,
            Self::Notify => 7,
        }
    }

    /// Position in [`CYCLE_ORDER`], or `None` for states that are not part of
    /// the cycle because the user never targets them directly.
    #[must_use]
    pub const fn cycle_slot(self) -> Option<usize> {
        let mut i = 0;
        while i < CYCLE_ORDER.len() {
            if matches!(
                (CYCLE_ORDER[i], self),
                (IslandState::Clock, IslandState::Clock)
                    | (IslandState::Media, IslandState::Media)
                    | (IslandState::Spectrum, IslandState::Spectrum)
                    | (IslandState::Metrics, IslandState::Metrics)
                    | (IslandState::Shortcuts, IslandState::Shortcuts)
                    | (IslandState::Notify, IslandState::Notify)
            ) {
                return Some(i);
            }
            i += 1;
        }
        None
    }

    /// The next state in cycle order.
    ///
    /// Wraps from the last entry to the first. [`IslandState::Collapsed`] and
    /// [`IslandState::Hover`] are not part of the cycle, so cycling into them
    /// from anywhere lands on the first real widget.
    #[must_use]
    pub const fn cycle_next(self) -> Self {
        // A state outside the cycle behaves as if it sat just before the first
        // entry, so the next state wraps to the first widget.
        let mut current: usize = CYCLE_ORDER.len() - 1;
        let mut i = 0;
        while i < CYCLE_ORDER.len() {
            if CYCLE_ORDER[i] as u8 == self as u8 {
                current = i;
            }
            i += 1;
        }
        CYCLE_ORDER[(current + 1) % CYCLE_ORDER.len()]
    }

    /// The previous state in cycle order, wrapping from first to last.
    #[must_use]
    pub const fn cycle_previous(self) -> Self {
        // A state outside the cycle behaves as if it were the first entry, so
        // the previous state is the last widget.
        let mut current: usize = 0;
        let mut i = 0;
        while i < CYCLE_ORDER.len() {
            if CYCLE_ORDER[i] as u8 == self as u8 {
                current = i;
            }
            i += 1;
        }
        CYCLE_ORDER[(current + CYCLE_ORDER.len() - 1) % CYCLE_ORDER.len()]
    }
}

impl std::fmt::Display for IslandState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.id())
    }
}

/// Where a state change came from, used to decide whether it is announced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransitionSource {
    /// Nobody asked, this is the arbiter settling.
    Automatic,
    /// The pointer entered or left.
    Pointer,
    /// A global or per-shortcut hotkey.
    Hotkey,
    /// The tray menu.
    Tray,
    /// An IPC client.
    Ipc,
    /// The user edited the configuration file.
    Config,
    /// A widget's own lifecycle, such as playback starting.
    Widget,
}

/// A widget's request to occupy the island.
///
/// Requests are owned by the orchestrator. A request carries an expiry measured
/// in monotonic milliseconds so the arbiter can be tested without a clock by
/// passing `now` explicitly.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StateRequest {
    /// The state being requested.
    pub state: IslandState,
    /// When the request expires, in monotonic milliseconds. `None` means it
    /// lives until explicitly cleared.
    pub expires_at_ms: Option<u64>,
    /// A pinned request is never dropped automatically, even if its state is no
    /// longer relevant. Used for tray and IPC driven states.
    pub pinned: bool,
    /// The state this request was in before, so a repeated request for the
    /// state the island is already showing does not restart the spring.
    pub source: TransitionSource,
}

impl StateRequest {
    /// A request that expires at the given monotonic timestamp.
    #[must_use]
    pub const fn until(state: IslandState, expires_at_ms: u64) -> Self {
        Self {
            state,
            expires_at_ms: Some(expires_at_ms),
            pinned: false,
            source: TransitionSource::Widget,
        }
    }

    /// A request that lives until cleared.
    ///
    /// Pinned requests form their own tier in the arbiter and outrank every
    /// automatic request, because a pin only ever comes from an explicit user
    /// action: a tray click, a hotkey, or an IPC command. A pinned request also
    /// never expires on its own.
    ///
    /// The orchestrator is responsible for clearing pins. `idle_timeout_ms` exists
    /// for exactly that: it releases pins once the pointer has been away long enough,
    /// so a pinned widget cannot block notifications forever.
    #[must_use]
    pub const fn persistent(state: IslandState, source: TransitionSource) -> Self {
        Self {
            state,
            expires_at_ms: None,
            pinned: true,
            source,
        }
    }

    /// A timed request from a specific source. The duration is applied later with
    /// [`StateRequest::with_duration_from`], because the constructor has no
    /// clock to measure from.
    #[must_use]
    pub const fn timed(state: IslandState, source: TransitionSource) -> Self {
        Self {
            state,
            expires_at_ms: None,
            pinned: false,
            source,
        }
    }

    /// Fill in the expiry relative to `now`.
    #[must_use]
    pub fn with_duration_from(self, now_ms: u64, duration_ms: u64) -> Self {
        Self {
            expires_at_ms: Some(now_ms.saturating_add(duration_ms)),
            ..self
        }
    }

    /// True when the request is still live at `now_ms`.
    #[must_use]
    pub fn is_live(&self, now_ms: u64) -> bool {
        self.pinned || self.expires_at_ms.is_none_or(|exp| now_ms < exp)
    }

    /// Sort key for the arbiter.
    ///
    /// Ordering is tier first, then priority, then remaining lifetime. The tier is
    /// what makes an explicit user pin beat a higher priority automatic request: the
    /// user asked for a specific widget, and a spectrum appearing at that moment
    /// should not snatch it away.
    fn sort_key(&self, now_ms: u64) -> (u8, u8, ReverseLifetime) {
        let remaining = self
            .expires_at_ms
            .map_or(u64::MAX, |e| e.saturating_sub(now_ms));
        let tier = u8::from(self.pinned);
        (tier, self.state.priority(), ReverseLifetime { remaining })
    }
}

/// Helper type so higher remaining lifetime sorts first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ReverseLifetime {
    remaining: u64,
}

impl Ord for ReverseLifetime {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        other.remaining.cmp(&self.remaining)
    }
}

impl PartialOrd for ReverseLifetime {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// The island's state, resolved.
///
/// Owns the request set and derives the current state from it. Deliberately has
/// no clock and no timers: the caller passes `now_ms`, which makes every rule
/// below directly testable.
#[derive(Debug, Clone, Default)]
pub struct StateMachine {
    current: IslandState,
    requests: Vec<StateRequest>,
    pointer_inside: bool,
    /// Set when the current state should be treated as sticky until the pointer
    /// leaves. Prevents a hover request from pulling the island out of a state
    /// the user pinned.
    hold_current: bool,
}

impl StateMachine {
    /// A machine resting in `Collapsed` with no requests.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The state currently being shown.
    #[must_use]
    pub fn current(&self) -> IslandState {
        self.current
    }

    /// True when the pointer is inside the island bounds.
    #[must_use]
    pub fn pointer_inside(&self) -> bool {
        self.pointer_inside
    }

    /// Live requests, in arbitrary order.
    #[must_use]
    pub fn requests(&self) -> &[StateRequest] {
        &self.requests
    }

    /// Submit or replace a request for `state`.
    ///
    /// Replacing rather than accumulating means a widget that re-announces
    /// itself every second does not build an unbounded list.
    pub fn request(&mut self, request: StateRequest) {
        match self.requests.iter_mut().find(|r| r.state == request.state) {
            Some(existing) => *existing = request,
            None => self.requests.push(request),
        }
    }

    /// Drop every request for `state`.
    pub fn clear(&mut self, state: IslandState) {
        self.requests.retain(|r| r.state != state);
    }

    /// Drop every request whose state is not in `keep`.
    pub fn retain_only(&mut self, keep: &[IslandState]) {
        self.requests.retain(|r| keep.contains(&r.state));
    }

    /// Drop every request. Used when the island is hidden.
    pub fn clear_all(&mut self) {
        self.requests.clear();
    }

    /// Record pointer presence, used for hover arbitration.
    pub fn set_pointer_inside(&mut self, inside: bool) {
        self.pointer_inside = inside;
    }

    /// Pin the current state until the pointer leaves or `release_hold` is
    /// called. Used so a hover request cannot yank the island out of a state the
    /// user is interacting with.
    pub fn hold(&mut self) {
        self.hold_current = true;
    }

    /// Release a hold previously set by [`StateMachine::hold`].
    pub fn release_hold(&mut self) {
        self.hold_current = false;
    }

    /// True when a hold is active.
    #[must_use]
    pub fn is_held(&self) -> bool {
        self.hold_current
    }

    /// Resolve the state that should be shown at `now_ms`.
    ///
    /// Pure with respect to time: the caller decides what "now" means. Rules,
    /// in order:
    ///
    /// 1. An explicit hold wins, because the user asked for it.
    /// 2. Expired requests are dropped.
    /// 3. Pinned requests beat unpinned ones, because a pin always comes from
    ///    an explicit user action. Within a tier, the highest priority wins,
    ///    ties broken by longest remaining lifetime.
    /// 4. If nothing is live and the pointer is inside, `Hover` is shown.
    /// 5. Otherwise `Collapsed`.
    ///
    /// Returns `true` when the resolved state differs from the current one,
    /// which is the signal the orchestrator uses to avoid restarting the spring
    /// on every frame.
    pub fn resolve(&mut self, now_ms: u64) -> bool {
        self.requests.retain(|r| r.is_live(now_ms));

        if self.hold_current {
            return self.apply(self.current);
        }

        let mut best: Option<&StateRequest> = None;
        for request in &self.requests {
            if request.state == IslandState::Collapsed || request.state == IslandState::Hover {
                continue;
            }
            let better = match best {
                None => true,
                Some(current) => request.sort_key(now_ms) > current.sort_key(now_ms),
            };
            if better {
                best = Some(request);
            }
        }

        let resolved = match best {
            Some(request) => request.state,
            None if self.pointer_inside => IslandState::Hover,
            None => IslandState::Collapsed,
        };

        self.apply(resolved)
    }

    /// Force the state, cancelling arbitration. Used by `--diagnostics` and by
    /// the initial layout before the first request arrives.
    pub fn force(&mut self, state: IslandState) {
        self.current = state;
    }

    /// Transition to the next state in cycle order, pinning it.
    ///
    /// This is what the `cycle_widget` hotkey does. Pinning matters: a user who
    /// presses the key expects to land on the widget they asked for, not to be
    /// immediately pulled back to whatever the audio pipeline is doing.
    pub fn cycle_forward(&mut self) -> IslandState {
        let next = self.current.cycle_next();
        self.request(StateRequest::persistent(next, TransitionSource::Hotkey));
        self.force(next);
        next
    }

    /// Cycle backwards through the widget states.
    pub fn cycle_backward(&mut self) -> IslandState {
        let next = self.current.cycle_previous();
        self.request(StateRequest::persistent(next, TransitionSource::Hotkey));
        self.force(next);
        next
    }

    /// Apply a resolved state, returning true when it changed.
    fn apply(&mut self, resolved: IslandState) -> bool {
        if self.current == resolved {
            return false;
        }
        self.current = resolved;
        true
    }
}

/// Normalized pointer state, in logical pixels relative to the window.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PointerState {
    /// Position in window space.
    pub position: crate::geometry::Vec2,
    /// True when over a hit testable element.
    pub over_hit_area: bool,
    /// True when a primary button is held.
    pub button_down: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 10_000;

    #[test]
    fn every_state_has_unique_id_and_parses_back() {
        for state in IslandState::ALL {
            assert_eq!(IslandState::from_id(state.id()), Some(state));
        }
        assert_eq!(
            IslandState::from_id("  Spectrum "),
            Some(IslandState::Spectrum)
        );
        assert_eq!(
            IslandState::from_id("SPECTRUM"),
            Some(IslandState::Spectrum)
        );
        assert_eq!(
            IslandState::from_id("spectrum"),
            Some(IslandState::Spectrum)
        );
        assert_eq!(IslandState::from_id("nonsense"), None);
        assert_eq!(IslandState::from_id(""), None);
    }

    #[test]
    fn every_state_geometry_is_a_valid_inverted_trapezoid() {
        for state in IslandState::ALL {
            let g = state.geometry();
            assert!(
                g.is_inverted_trapezoid(),
                "{state} is not an inverted trapezoid: {g:?}"
            );
            assert!(g.height > 0.0, "{state} has no height");
            assert!(g.radius >= 0.0, "{state} has a negative radius");
            assert!(
                g.effective_radius() > 0.0,
                "{state} has a degenerate radius"
            );
        }
    }

    #[test]
    fn state_geometries_are_distinct() {
        // Two states sharing geometry would make a transition invisible, which
        // reads as a bug even though it is not one.
        let mut seen: Vec<(u32, u32)> = IslandState::ALL
            .iter()
            .map(|s| {
                let g = s.geometry();
                (g.half_top.to_bits(), g.height.to_bits())
            })
            .collect();
        let total = seen.len();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), total, "two states share geometry");
    }

    #[test]
    fn priority_is_strictly_ordered_as_designed() {
        let ordered = [
            IslandState::Collapsed,
            IslandState::Hover,
            IslandState::Clock,
            IslandState::Metrics,
            IslandState::Shortcuts,
            IslandState::Media,
            IslandState::Spectrum,
            IslandState::Notify,
        ];
        for pair in ordered.windows(2) {
            assert!(
                pair[0].priority() < pair[1].priority(),
                "{} should rank below {}",
                pair[0],
                pair[1]
            );
        }
    }

    #[test]
    fn starts_collapsed() {
        let mut sm = StateMachine::new();
        assert_eq!(sm.current(), IslandState::Collapsed);
        assert!(
            !sm.resolve(NOW),
            "resolving an idle machine is not a change"
        );
    }

    #[test]
    fn single_request_wins() {
        let mut sm = StateMachine::new();
        sm.request(StateRequest::until(IslandState::Media, NOW + 5_000));
        assert!(sm.resolve(NOW));
        assert_eq!(sm.current(), IslandState::Media);
    }

    #[test]
    fn higher_priority_wins_regardless_of_request_order() {
        // Submit the low priority first so insertion order cannot be the reason
        // the winner is correct.
        let mut sm = StateMachine::new();
        sm.request(StateRequest::until(IslandState::Clock, NOW + 9_000));
        sm.request(StateRequest::until(IslandState::Notify, NOW + 1_000));
        sm.resolve(NOW);
        assert_eq!(sm.current(), IslandState::Notify);

        // And the reverse order.
        let mut sm2 = StateMachine::new();
        sm2.request(StateRequest::until(IslandState::Notify, NOW + 1_000));
        sm2.request(StateRequest::until(IslandState::Clock, NOW + 9_000));
        sm2.resolve(NOW);
        assert_eq!(sm2.current(), IslandState::Notify);
    }

    #[test]
    fn equal_priority_breaks_tie_toward_longer_lifetime() {
        let mut sm = StateMachine::new();
        sm.request(StateRequest::until(IslandState::Media, NOW + 1_000));
        sm.request(StateRequest::until(IslandState::Spectrum, NOW + 9_000));
        sm.resolve(NOW);
        assert_eq!(
            sm.current(),
            IslandState::Spectrum,
            "longer lived should win the tie"
        );
    }

    #[test]
    fn expired_requests_are_dropped_and_reveal_the_next_one_down() {
        let mut sm = StateMachine::new();
        sm.request(StateRequest::until(IslandState::Notify, NOW + 1_000));
        sm.request(StateRequest::until(IslandState::Media, NOW + 30_000));
        sm.resolve(NOW);
        assert_eq!(sm.current(), IslandState::Notify);

        // After the notification expires, media takes over rather than the
        // island snapping shut.
        sm.resolve(NOW + 2_000);
        assert_eq!(sm.current(), IslandState::Media);
        assert_eq!(sm.requests().len(), 1, "expired request should be gone");
    }

    #[test]
    fn all_requests_expired_falls_back_to_collapsed() {
        let mut sm = StateMachine::new();
        sm.request(StateRequest::until(IslandState::Notify, NOW + 500));
        sm.resolve(NOW);
        assert_eq!(sm.current(), IslandState::Notify);
        sm.resolve(NOW + 1_000);
        assert_eq!(sm.current(), IslandState::Collapsed);
        assert_eq!(sm.requests(), []);
    }

    #[test]
    fn pointer_inside_shows_hover_only_when_nothing_else_is_live() {
        let mut sm = StateMachine::new();
        sm.set_pointer_inside(true);
        sm.resolve(NOW);
        assert_eq!(sm.current(), IslandState::Hover);

        sm.request(StateRequest::until(IslandState::Media, NOW + 5_000));
        sm.resolve(NOW);
        assert_eq!(sm.current(), IslandState::Media, "media must outrank hover");

        sm.clear(IslandState::Media);
        sm.resolve(NOW);
        assert_eq!(sm.current(), IslandState::Hover);
    }

    #[test]
    fn hold_pins_the_current_state_against_arbitration() {
        let mut sm = StateMachine::new();
        sm.request(StateRequest::until(IslandState::Media, NOW + 10_000));
        sm.resolve(NOW);
        assert_eq!(sm.current(), IslandState::Media);

        sm.hold();
        sm.request(StateRequest::until(IslandState::Notify, NOW + 10_000));
        sm.resolve(NOW);
        assert_eq!(
            sm.current(),
            IslandState::Media,
            "hold must resist a higher priority request"
        );

        sm.release_hold();
        sm.resolve(NOW);
        assert_eq!(sm.current(), IslandState::Notify);
    }

    #[test]
    fn repeated_request_replaces_rather_than_accumulates() {
        let mut sm = StateMachine::new();
        for _ in 0..1_000 {
            sm.request(StateRequest::until(IslandState::Media, NOW + 5_000));
        }
        assert_eq!(sm.requests().len(), 1);
    }

    #[test]
    fn pinned_requests_never_expire() {
        let mut sm = StateMachine::new();
        sm.request(StateRequest::persistent(
            IslandState::Shortcuts,
            TransitionSource::Tray,
        ));
        sm.resolve(NOW);
        assert_eq!(sm.current(), IslandState::Shortcuts);

        sm.resolve(NOW + 1_000_000_000);
        assert_eq!(sm.current(), IslandState::Shortcuts);
        assert_eq!(sm.requests().len(), 1);
    }

    #[test]
    fn cycling_visits_every_content_state_and_wraps() {
        let mut sm = StateMachine::new();
        let mut seen = vec![sm.current()];
        for _ in 0..6 {
            seen.push(sm.cycle_forward());
        }
        for state in [
            IslandState::Clock,
            IslandState::Media,
            IslandState::Spectrum,
            IslandState::Metrics,
            IslandState::Shortcuts,
            IslandState::Notify,
        ] {
            assert!(seen.contains(&state), "cycle skipped {state}");
        }
        assert_eq!(sm.cycle_forward(), IslandState::Clock, "cycle should wrap");
    }

    #[test]
    fn backward_cycle_is_the_inverse_of_forward() {
        let mut sm = StateMachine::new();
        for _ in 0..4 {
            sm.cycle_forward();
        }
        let after_forward = sm.current();
        let back = sm.cycle_backward();
        sm.cycle_forward();
        assert_eq!(
            sm.current(),
            after_forward,
            "forward then back then forward must return"
        );
        let _ = back;
    }

    #[test]
    fn cycling_pins_so_the_island_does_not_snap_away() {
        let mut sm = StateMachine::new();
        sm.cycle_forward();
        let chosen = sm.current();
        // Something loud happens while the island is idle.
        sm.request(StateRequest::until(IslandState::Media, NOW + 5_000));
        sm.resolve(NOW);
        assert_eq!(sm.current(), chosen, "user initiated cycle must survive");
    }

    #[test]
    fn resolve_reports_no_change_when_nothing_moves() {
        let mut sm = StateMachine::new();
        sm.request(StateRequest::until(IslandState::Media, NOW + 5_000));
        assert!(sm.resolve(NOW), "first resolve is a change");
        for _ in 0..10 {
            assert!(
                !sm.resolve(NOW),
                "steady state resolve must report no change"
            );
        }
    }

    #[test]
    fn resolve_is_pure_with_respect_to_time() {
        // The same request set at the same logical instant always resolves the
        // same way, no matter how the clock got there.
        let build = || {
            let mut sm = StateMachine::new();
            sm.request(StateRequest::until(IslandState::Metrics, NOW + 4_000));
            sm.request(StateRequest::until(IslandState::Media, NOW + 8_000));
            sm
        };
        let a = {
            let mut sm = build();
            sm.resolve(NOW);
            sm.current()
        };
        let b = {
            let mut sm = build();
            sm.resolve(NOW);
            sm.current()
        };
        assert_eq!(a, b);
        assert_eq!(a, IslandState::Media);
    }

    #[test]
    fn requests_for_collapsed_and_hover_are_never_selected() {
        let mut sm = StateMachine::new();
        sm.request(StateRequest::persistent(
            IslandState::Collapsed,
            TransitionSource::Ipc,
        ));
        sm.request(StateRequest::persistent(
            IslandState::Hover,
            TransitionSource::Ipc,
        ));
        sm.resolve(NOW);
        assert_eq!(sm.current(), IslandState::Collapsed);
    }

    #[test]
    fn retain_only_drops_everything_else() {
        let mut sm = StateMachine::new();
        sm.request(StateRequest::persistent(
            IslandState::Media,
            TransitionSource::Widget,
        ));
        sm.request(StateRequest::persistent(
            IslandState::Clock,
            TransitionSource::Widget,
        ));
        sm.retain_only(&[IslandState::Clock]);
        assert_eq!(sm.requests().len(), 1);
        assert_eq!(sm.requests()[0].state, IslandState::Clock);
    }

    #[test]
    fn clear_all_returns_to_collapsed() {
        let mut sm = StateMachine::new();
        sm.request(StateRequest::persistent(
            IslandState::Media,
            TransitionSource::Widget,
        ));
        sm.resolve(NOW);
        sm.clear_all();
        sm.resolve(NOW);
        assert_eq!(sm.current(), IslandState::Collapsed);
    }

    #[test]
    fn content_opacity_is_zero_only_when_contentless() {
        for state in IslandState::ALL {
            let opacity = state.content_opacity();
            assert!(
                (0.0..=1.0).contains(&opacity),
                "{state} opacity out of range"
            );
            if state.is_contentless() {
                assert_eq!(opacity, 0.0, "{state} is contentless but has opacity");
            } else {
                assert!(
                    opacity > 0.0,
                    "{state} shows content but is fully transparent"
                );
            }
        }
    }

    #[test]
    fn display_matches_id() {
        for state in IslandState::ALL {
            assert_eq!(state.to_string(), state.id());
        }
    }

    #[test]
    fn serializes_as_snake_case_for_config_and_ipc() {
        let json = serde_json::to_string(&IslandState::Spectrum).expect("serialize");
        assert_eq!(json, "\"spectrum\"");
        let back: IslandState = serde_json::from_str("\"notify\"").expect("deserialize");
        assert_eq!(back, IslandState::Notify);
    }
}

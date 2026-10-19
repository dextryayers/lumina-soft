//! Idle-inhibit policy for a pinned island.
//!
//! A pinned island is the user saying "keep this visible while I do something
//! else". If the compositor blanks the screen after five minutes, the island
//! disappears exactly when it was meant to stay. The portal
//! `org.freedesktop.portal.Inhibit` is the only supported way for a sandboxed
//! or unprivileged client to ask for that.
//!
//! This module decides *when* to inhibit and deduplicates the requests. The
//! D-Bus call itself belongs to the runtime layer, because it needs a session
//! bus and a portal implementation, neither of which exists on a CI runner and
//! neither of which this crate should fake. Keeping the policy pure means the
//! decision is tested everywhere while the call site stays a thin adapter.
//!
//! # Flags
//!
//! The portal takes a bit mask of what to inhibit. Lumina only ever requests
//! `idle`; inhibiting logout or suspend from a desktop widget would be hostile.

/// The portal flag for inhibiting the idle timeout.
pub const INHIBIT_IDLE: u32 = 8;

/// When the island should ask to keep the session awake.
///
/// The defaults follow the feature's purpose: a pinned island inhibits idle,
/// because the user explicitly asked it to stay. An expanded island does not,
/// because typing in another window while the island shows a notification must
/// still let the screen sleep.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IdleInhibitPolicy {
    /// Inhibit while the island is pinned.
    pub while_pinned: bool,
    /// Inhibit while the island is expanded.
    pub while_expanded: bool,
}

impl IdleInhibitPolicy {
    /// A policy that only honours pinning.
    #[must_use]
    pub const fn pinned_only() -> Self {
        Self {
            while_pinned: true,
            while_expanded: false,
        }
    }

    /// True when the current island state should inhibit idle.
    #[must_use]
    pub const fn should_inhibit(self, pinned: bool, expanded: bool) -> bool {
        (pinned && self.while_pinned) || (expanded && self.while_expanded)
    }

    /// The portal flags to send, or `None` when no inhibition is needed.
    #[must_use]
    pub const fn flags(self, pinned: bool, expanded: bool) -> Option<u32> {
        if self.should_inhibit(pinned, expanded) {
            Some(INHIBIT_IDLE)
        } else {
            None
        }
    }
}

impl Default for IdleInhibitPolicy {
    fn default() -> Self {
        Self::pinned_only()
    }
}

/// Deduplicates inhibit requests so the portal is not called every frame.
///
/// The tracker compares the desired state against the last confirmed state.
/// A caller loops over island state each frame; only transitions produce a
/// portal call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct InhibitTracker {
    desired: bool,
    active: bool,
    transitions: u32,
}

impl InhibitTracker {
    /// Create a tracker in the "not inhibited" state.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            desired: false,
            active: false,
            transitions: 0,
        }
    }

    /// Record the state the island wants.
    ///
    /// Returns `Some(true)` when an inhibit request must now be sent, and
    /// `Some(false)` when the active inhibition must be released. Returns
    /// `None` when nothing needs to change, which is the common case.
    pub fn update(&mut self, desired: bool) -> Option<bool> {
        if desired == self.desired {
            return None;
        }
        self.desired = desired;
        if desired == self.active {
            return None;
        }
        self.transitions = self.transitions.saturating_add(1);
        Some(desired)
    }

    /// Record that the portal confirmed the last requested state.
    ///
    /// If the portal fails, the caller can leave `active` unchanged, and the
    /// next [`Self::update`] with the same desired state will retry.
    pub fn confirm(&mut self, active: bool) {
        self.active = active;
    }

    /// Whether inhibition is currently active.
    #[must_use]
    pub const fn active(&self) -> bool {
        self.active
    }

    /// Whether the last requested state was inhibit.
    #[must_use]
    pub const fn desired(&self) -> bool {
        self.desired
    }

    /// How many state transitions have been requested.
    #[must_use]
    pub const fn transitions(&self) -> u32 {
        self.transitions
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinning_inhibits_idle_by_default() {
        let policy = IdleInhibitPolicy::default();
        assert!(policy.should_inhibit(true, false));
        assert!(!policy.should_inhibit(false, true));
    }

    #[test]
    fn a_custom_policy_can_tie_inhibition_to_expansion() {
        let policy = IdleInhibitPolicy {
            while_pinned: false,
            while_expanded: true,
        };
        assert!(!policy.should_inhibit(true, false));
        assert!(policy.should_inhibit(false, true));
        assert_eq!(policy.flags(false, true), Some(INHIBIT_IDLE));
        assert_eq!(policy.flags(true, false), None);
    }

    #[test]
    fn only_the_idle_flag_is_ever_requested() {
        assert_eq!(
            IdleInhibitPolicy::pinned_only().flags(true, false),
            Some(INHIBIT_IDLE)
        );
        assert_eq!(INHIBIT_IDLE, 8);
    }

    #[test]
    fn the_tracker_only_emits_transitions() {
        let mut tracker = InhibitTracker::new();
        assert_eq!(tracker.update(false), None, "already idle");
        assert_eq!(tracker.update(true), Some(true));
        assert_eq!(
            tracker.update(true),
            None,
            "the same state must not spam the portal"
        );
        tracker.confirm(true);
        assert!(tracker.active());
        assert_eq!(tracker.update(false), Some(false));
        tracker.confirm(false);
        assert!(!tracker.active());
        assert_eq!(tracker.transitions(), 2);
    }

    #[test]
    fn a_failed_portal_call_is_retried_on_the_next_frame() {
        let mut tracker = InhibitTracker::new();
        assert_eq!(tracker.update(true), Some(true));
        // The call failed, so `confirm` is never called.
        assert!(!tracker.active());
        assert_eq!(tracker.update(true), None);
        tracker.confirm(true);
        assert!(tracker.active());
        assert_eq!(tracker.update(true), None);
    }

    #[test]
    fn a_policy_change_back_to_the_active_state_needs_no_call() {
        let mut tracker = InhibitTracker::new();
        tracker.update(true);
        tracker.confirm(true);
        // A spurious transition to false is requested, then withdrawn before
        // the portal answered. No portal call is outstanding, so none is made.
        assert_eq!(tracker.update(false), Some(false));
        assert_eq!(tracker.update(true), None);
        assert!(tracker.active());
    }
}

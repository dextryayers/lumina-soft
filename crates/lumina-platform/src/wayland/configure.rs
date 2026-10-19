//! The layer-surface configure handshake, as an explicit state machine.
//!
//! A Wayland surface only becomes mapped after the client asks for a
//! configure, receives one, applies the size, and acknowledges the serial. The
//! order is not advisory: acknowledging before the swapchain has the new size
//! produces a frame at the old size, which users see as a flicker or a black
//! band, and acknowledging a serial the compositor has already replaced can
//! stall the surface entirely.
//!
//! [`ConfigureTracker`] owns the serial bookkeeping and refuses out-of-order
//! events. The backend still performs the protocol calls, but it cannot get the
//! ordering wrong because the only way to obtain an acknowledgement is through
//! [`ConfigureTracker::ack`], which is only available after a configure has been
//! received.
//!
//! # Zero sizes
//!
//! Both layer-shell and xdg-shell use zero width or height to mean "the client
//! should choose". A tracker therefore substitutes its preferred size, which is
//! the size the island wants for its current state. A preferred size of zero is
//! invalid for the same reason a zero-sized window is: every compositor does
//! something different with it, and none of those things is useful.

/// The smallest surface extent accepted from configuration.
pub const MIN_EXTENT: u32 = 1;

/// A configure event that has been received but not acknowledged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingConfigure {
    /// The serial to acknowledge.
    pub serial: u32,
    /// The surface size after substituting the preferred size for zero.
    pub size: (u32, u32),
    /// True when this size differs from the last applied one.
    pub resized: bool,
}

/// A configure event that has been applied and acknowledged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppliedConfigure {
    /// The acknowledged serial.
    pub serial: u32,
    /// The size that was applied.
    pub size: (u32, u32),
    /// True when the size changed.
    pub resized: bool,
}

/// Why a configure event was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigureError {
    /// The serial is not newer than the last one seen.
    ///
    /// Wayland serials increase, so an equal or lower value is either a
    /// duplicate delivered twice or an event from before a resize that has
    /// already been handled. Acknowledging it would move the surface backwards.
    StaleSerial {
        /// The refused serial.
        serial: u32,
        /// The highest serial already seen.
        last: u32,
    },
}

impl std::fmt::Display for ConfigureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::StaleSerial { serial, last } => {
                write!(f, "stale configure serial {serial}, last was {last}")
            }
        }
    }
}

impl std::error::Error for ConfigureError {}

/// Tracks configure serials and applied sizes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigureTracker {
    preferred: (u32, u32),
    configured: (u32, u32),
    last_serial: Option<u32>,
    pending: Option<PendingConfigure>,
    acked: u32,
}

impl ConfigureTracker {
    /// Create a tracker with the size the island would like to have.
    ///
    /// A zero extent is raised to [`MIN_EXTENT`] rather than accepted, because
    /// a zero-sized surface breaks every compositor in a different way.
    #[must_use]
    pub const fn new(preferred: (u32, u32)) -> Self {
        let preferred = sanitize(preferred);
        Self {
            preferred,
            configured: preferred,
            last_serial: None,
            pending: None,
            acked: 0,
        }
    }

    /// The size configured on the surface.
    #[must_use]
    pub const fn configured_size(&self) -> (u32, u32) {
        self.configured
    }

    /// The size the island wants, used when the compositor sends zero.
    #[must_use]
    pub const fn preferred_size(&self) -> (u32, u32) {
        self.preferred
    }

    /// Change the preferred size.
    ///
    /// Returns true when the value actually changed. The caller is responsible
    /// for sending a new `set_size` and committing, which asks the compositor
    /// for another configure. The currently pending configure, if any, is left
    /// alone: it is still the size the surface must use until the compositor
    /// answers the new request.
    pub fn set_preferred(&mut self, preferred: (u32, u32)) -> bool {
        let preferred = sanitize(preferred);
        if preferred == self.preferred {
            return false;
        }
        self.preferred = preferred;
        true
    }

    /// The highest serial seen so far.
    #[must_use]
    pub const fn last_serial(&self) -> Option<u32> {
        self.last_serial
    }

    /// How many configures have been acknowledged.
    #[must_use]
    pub const fn acked_count(&self) -> u32 {
        self.acked
    }

    /// The configure waiting to be acknowledged, if any.
    #[must_use]
    pub const fn pending(&self) -> Option<PendingConfigure> {
        self.pending
    }

    /// Record a configure event from the compositor.
    ///
    /// The returned value describes the geometry to apply. The caller must call
    /// [`Self::ack`] only after the renderer has adopted `size`, and only once
    /// per pending configure.
    pub fn on_configure(
        &mut self,
        serial: u32,
        width: u32,
        height: u32,
    ) -> Result<PendingConfigure, ConfigureError> {
        if let Some(last) = self.last_serial {
            if serial <= last {
                return Err(ConfigureError::StaleSerial { serial, last });
            }
        }
        self.last_serial = Some(serial);

        let size = if width == 0 || height == 0 {
            self.preferred
        } else {
            (width, height)
        };
        let pending = PendingConfigure {
            serial,
            size,
            resized: size != self.configured,
        };
        self.pending = Some(pending);
        Ok(pending)
    }

    /// Acknowledge the pending configure.
    ///
    /// Returns `None` when there is nothing pending, which is the correct
    /// response to a duplicate acknowledgement: the surface has already been
    /// acknowledged at this serial, and sending the request twice is a protocol
    /// error.
    pub fn ack(&mut self) -> Option<AppliedConfigure> {
        let pending = self.pending.take()?;
        self.configured = pending.size;
        self.acked = self.acked.saturating_add(1);
        Some(AppliedConfigure {
            serial: pending.serial,
            size: pending.size,
            resized: pending.resized,
        })
    }

    /// True when a configure has been received and not yet acknowledged.
    #[must_use]
    pub const fn needs_ack(&self) -> bool {
        self.pending.is_some()
    }
}

/// Replace zero extents with the minimum usable value.
const fn sanitize(size: (u32, u32)) -> (u32, u32) {
    (
        if size.0 < MIN_EXTENT {
            MIN_EXTENT
        } else {
            size.0
        },
        if size.1 < MIN_EXTENT {
            MIN_EXTENT
        } else {
            size.1
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_zero_extent_uses_the_preferred_size() {
        let mut tracker = ConfigureTracker::new((320, 36));
        let pending = tracker
            .on_configure(1, 0, 0)
            .expect("the first configure is always accepted");
        assert_eq!(pending.size, (320, 36));
        assert!(!pending.resized, "the size is already the configured one");
    }

    #[test]
    fn a_partial_zero_extent_uses_the_preferred_size_for_both_axes() {
        // Layer-shell sends either both extents or neither in practice, but
        // treating a mixed event as "client chooses" is safer than applying a
        // half-known size.
        let mut tracker = ConfigureTracker::new((320, 36));
        let pending = tracker.on_configure(1, 640, 0).expect("accepted");
        assert_eq!(pending.size, (320, 36));
    }

    #[test]
    fn an_explicit_extent_is_applied_verbatim() {
        let mut tracker = ConfigureTracker::new((320, 36));
        let pending = tracker.on_configure(7, 800, 48).expect("accepted");
        assert_eq!(pending.serial, 7);
        assert_eq!(pending.size, (800, 48));
        assert!(pending.resized);
    }

    #[test]
    fn the_configured_size_only_changes_after_acknowledgement() {
        let mut tracker = ConfigureTracker::new((320, 36));
        tracker.on_configure(3, 800, 48).expect("accepted");
        assert_eq!(
            tracker.configured_size(),
            (320, 36),
            "receiving a configure must not silently apply it"
        );
        assert!(tracker.needs_ack());
        let applied = tracker.ack().expect("a pending configure exists");
        assert_eq!(applied.serial, 3);
        assert_eq!(tracker.configured_size(), (800, 48));
        assert!(!tracker.needs_ack());
    }

    #[test]
    fn a_serial_that_does_not_advance_is_refused() {
        let mut tracker = ConfigureTracker::new((320, 36));
        tracker.on_configure(5, 320, 36).expect("accepted");
        tracker.ack();
        let error = tracker
            .on_configure(5, 400, 36)
            .expect_err("a duplicate serial must be refused");
        assert_eq!(error, ConfigureError::StaleSerial { serial: 5, last: 5 });
        let error = tracker
            .on_configure(4, 400, 36)
            .expect_err("an older serial must be refused");
        assert_eq!(error, ConfigureError::StaleSerial { serial: 4, last: 5 });
    }

    #[test]
    fn a_duplicate_configure_before_ack_is_also_refused() {
        // Two events with one serial would otherwise be acknowledged twice,
        // which is a protocol violation and stalls the surface.
        let mut tracker = ConfigureTracker::new((320, 36));
        tracker.on_configure(9, 320, 36).expect("accepted");
        assert!(tracker.on_configure(9, 320, 36).is_err());
        tracker.ack();
        assert!(tracker.ack().is_none(), "one ack per configure");
    }

    #[test]
    fn a_newer_configure_replaces_an_unanswered_one() {
        let mut tracker = ConfigureTracker::new((320, 36));
        tracker.on_configure(10, 300, 36).expect("accepted");
        let newer = tracker.on_configure(11, 360, 40).expect("accepted");
        assert_eq!(newer.serial, 11);
        assert_eq!(newer.size, (360, 40));
        let applied = tracker.ack().expect("pending");
        assert_eq!(
            applied.serial, 11,
            "the newest event is the one that gets acknowledged"
        );
        assert_eq!(tracker.configured_size(), (360, 40));
    }

    #[test]
    fn a_preferred_size_of_zero_is_raised_to_the_minimum() {
        let tracker = ConfigureTracker::new((0, 0));
        assert_eq!(tracker.preferred_size(), (MIN_EXTENT, MIN_EXTENT));
        assert_eq!(tracker.configured_size(), (MIN_EXTENT, MIN_EXTENT));
    }

    #[test]
    fn changing_the_preferred_size_is_reported_once() {
        let mut tracker = ConfigureTracker::new((320, 36));
        assert!(!tracker.set_preferred((320, 36)));
        assert!(tracker.set_preferred((480, 36)));
        assert_eq!(tracker.preferred_size(), (480, 36));
        assert_eq!(
            tracker.configured_size(),
            (320, 36),
            "the surface keeps its current size until the compositor answers"
        );
    }

    #[test]
    fn acknowledging_nothing_is_not_an_error() {
        let mut tracker = ConfigureTracker::new((320, 36));
        assert!(tracker.ack().is_none());
        assert_eq!(tracker.acked_count(), 0);
    }

    #[test]
    fn stale_errors_render_readable_messages() {
        let error = ConfigureError::StaleSerial {
            serial: 4,
            last: 9,
        };
        let text = error.to_string();
        assert!(text.contains('4'));
        assert!(text.contains('9'));
        assert!(!text.contains('\u{2014}'));
    }
}

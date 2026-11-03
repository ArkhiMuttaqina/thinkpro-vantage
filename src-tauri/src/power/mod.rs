//! Power management: who owns each knob, and what this app may touch.
//!
//! TLP, power-profiles-daemon, LACT and scx_loader all write the same sysfs
//! nodes. Writing a knob another daemon owns is not a no-op: the value is
//! either reverted on that daemon's next tick or it fights for the node, which
//! is how a machine ends up pinned to power-saver in three independent places.
//!
//! Everything here answers the question *before* a write: which daemon holds
//! this knob, what breaks if we ignore it, and how much of the app is usable
//! given what is installed.

pub mod arbiter;
pub mod doctor;
pub mod tlp;

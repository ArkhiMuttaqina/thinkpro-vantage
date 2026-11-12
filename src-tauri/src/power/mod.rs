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
//!
//! For most knobs the answer is a read of the system. For the GPU it is an
//! action: LACT owns the amdgpu level wherever lactd runs and re-applies it, so
//! `lact` is the one module that asks another daemon to step aside rather than
//! only detecting that it is there.

pub mod arbiter;
pub mod doctor;
pub mod lact;
pub mod tlp;
pub mod writer;

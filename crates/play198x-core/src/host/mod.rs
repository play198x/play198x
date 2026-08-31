//! A host: RAM, a CPU, a clock and a sound chip, assembled to run someone
//! else's code. Not a machine — no keyboard, no display, no disk, no ROM.

#[cfg(feature = "sid")]
pub mod c64;
#[cfg(feature = "ay")]
pub mod memory;
#[cfg(feature = "ay")]
pub mod spectrum;

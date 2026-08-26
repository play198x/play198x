//! Turn identified bytes into something an interface can show or play.
//!
//! The Format198x crates stay index-only, because a palette belongs to
//! `mediaspec198x` and a dependency-free format crate cannot reach it. The
//! conversion to RGBA therefore happens here, where the palette is in hand.

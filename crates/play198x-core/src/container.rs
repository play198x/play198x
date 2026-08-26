//! Open a path, enumerate its entries, and read one back.
//!
//! A plain file, a ZIP archive and an Amiga ADF disk image all present the same
//! way, so probing and decoding take one code path regardless of where the
//! bytes came from.

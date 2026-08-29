//! Identify a format from its bytes, and say how sure that identification is.
//!
//! Identification is from the bytes alone. Extensions on this material are
//! unreliable to the point of being noise — a module arrives as `.mod`, `.MOD`,
//! `mod.something`, or with no extension at all — so nothing here looks at a
//! name, and [`identify`] is not given one.
//!
//! # The signals, strongest first
//!
//! | Format | Signal | Confidence |
//! |---|---|---|
//! | ProTracker | a recognised magic at offset 1080 | [`Confidence::Certain`] |
//! | ILBM | `FORM` at 0 and `ILBM` at 8 | [`Confidence::Certain`] |
//! | `.ay` | `ZXAY` at 0 and `EMUL` at 4 | [`Confidence::Certain`] |
//! | Koala | load address `0x6000` **and** length 10,003 | [`Confidence::Certain`] |
//! | Art Studio | load address `0x2000` **and** length 9,002..=9,009 | [`Confidence::Probable`] |
//! | SCR | length exactly 6,912, and nothing above matched | [`Confidence::Probable`] |
//!
//! **What separates the two halves of that table is whether a miss can be
//! caught.** The three certain formats carry a magic number or a checksum, so
//! bytes that are not one of them fail to match rather than matching wrongly.
//! The two probable ones have neither: a load address and a length are the
//! whole signal, and plenty of files that are not Art Studio bitmaps begin
//! `0x2000` and run to 9,009 bytes.
//!
//! That is the fact a caller needs. A wrong `Probable` does not surface as an
//! error — the decoder accepts the bytes and produces a picture that simply
//! looks wrong — so an interface built on this should say when an
//! identification is weak rather than presenting it as settled.
//!
//! The order is the substance, not an implementation detail. SCR is identified
//! by its length and nothing else, so it must be tried last: an ILBM or a
//! module that happens to run to 6,912 bytes is neither rare nor contrived, and
//! testing the weak rule first would hand back a Spectrum screen for both.
//!
//! # Why a confidence and not a bare answer
//!
//! Five of the six rules rest on a magic number or on two independent facts
//! agreeing. The sixth rests on a file being 6,912 bytes long, which is a
//! property any 6,912 bytes have. That is genuinely weak evidence and the
//! return type says so, so an interface can present a `Probable` result
//! differently — offer it, rather than assert it — instead of every caller
//! having to remember which formats are guesses.
//!
//! # Known gap, deliberately left
//!
//! A self-extracting archive — a ZIP with an executable stub in front of it —
//! is missed by [`crate::container`]'s four-byte sniff, because a ZIP is found
//! from its tail rather than its head, while `zip::ZipArchive::new` parses one
//! happily. SFX `.exe` archives are common for this material. Catching them
//! means scanning the tail for an end-of-central-directory record, which is new
//! capability rather than a bug fix, and it belongs here beside the other
//! from-the-bytes identification rather than in the container's cheap sniff.
//! Recorded so it is a known absence rather than a silent one.

use format198x_commodore_amiga_mod::is_module;

/// A format this crate can decode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Format {
    /// Sinclair ZX Spectrum screen memory dump.
    Scr,
    /// Commodore 64 Koala Painter multicolour bitmap.
    Koala,
    /// Commodore 64 Advanced Art Studio high-resolution bitmap.
    ArtStudio,
    /// Amiga IFF interleaved bitmap.
    Ilbm,
    /// ProTracker module.
    ProTracker,
    /// ZXAY/EMUL `.ay` tune: Z80 code and data for a 128K Spectrum host to
    /// run, not sample data. Identifying one needs only its eight-byte
    /// header, so this variant and the rule below are not behind the `ay`
    /// feature that its player and its Z80/AY host are — see `player::ay`.
    Ay,
}

/// How much the evidence behind an identification is worth.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confidence {
    /// A magic number or structural signature identifies this and nothing else.
    Certain,
    /// The bytes are consistent with this format, but the evidence is weak —
    /// a length or a load address that other things also have.
    Probable,
}

/// Work out what `bytes` are.
///
/// Returns `None` when nothing matches, which includes an empty slice: this
/// takes arbitrary bytes by definition and never panics on any of them, because
/// a panic reaching the FFI boundary this crate is built for is undefined
/// behaviour rather than a crash.
#[must_use]
pub fn identify(bytes: &[u8]) -> Option<(Format, Confidence)> {
    // Strongest first. A magic number at a fixed offset identifies one format
    // and nothing else, so these two can never be wrong about a file that
    // merely has a coincidental length.
    if is_module(bytes) {
        return Some((Format::ProTracker, Confidence::Certain));
    }

    if is_ilbm(bytes) {
        return Some((Format::Ilbm, Confidence::Certain));
    }

    // `ZXAY` plus `EMUL`, eight bytes of magic in fixed positions — the same
    // eight bytes `player::ay::format::parse` checks to accept the file.
    // Placed with the other magic checks, above every length-only rule, for
    // the same reason ILBM is: a rule that reads real structure must not be
    // shadowed by one that reads a file size.
    //
    // Not behind the `ay` feature, unlike everything that plays one. Naming
    // this file costs eight byte comparisons; nothing here touches a Z80 or
    // an AY chip. A default build that could plainly say ".ay" and instead
    // reported nothing would be the exact silent failure this module's
    // confidence system exists to avoid.
    if bytes.len() >= 8 && &bytes[0..4] == b"ZXAY" && &bytes[4..8] == b"EMUL" {
        return Some((Format::Ay, Confidence::Certain));
    }

    // Two independent facts each. A C64 file's first two bytes are the load
    // address the KERNAL will use, which on its own is far too common to mean
    // anything — plenty of files start `00 60`. Paired with the exact length
    // the format produces, it is decisive.
    let load_address = load_address(bytes);

    if load_address == Some(format198x_commodore_c64_koala::LOAD_ADDRESS)
        && bytes.len() == format198x_commodore_c64_koala::FILE_LEN
    {
        return Some((Format::Koala, Confidence::Certain));
    }

    // Art Studio's trailing seven-byte pad is optional in the wild, so the
    // length is a small range rather than a single number. Both ends come from
    // the crate that decodes it.
    //
    // `Probable`, not `Certain`, and the reason is what happens downstream
    // rather than the strength of the two signals here. `$2000` is about the
    // commonest load address on the C64, so the evidence is a very ordinary
    // address plus an eight-byte-wide length window. That would be tolerable if
    // anything later could catch a miss — but the Art Studio decoder checks
    // exactly these two facts and nothing else, so a file that is not one
    // decodes *successfully* into a wrong-looking picture. There is no second
    // check anywhere in the pipeline.
    //
    // Reporting it as `Probable` is the only place that doubt can be expressed,
    // and it lets a shell show the result with a caveat instead of asserting a
    // picture it cannot stand behind. Koala earns `Certain` on the same shape
    // of evidence because `$6000` is unusual and its length is exact.
    if load_address == Some(format198x_commodore_c64_art_studio::LOAD_ADDRESS)
        && (format198x_commodore_c64_art_studio::MIN_FILE_LEN
            ..=format198x_commodore_c64_art_studio::FILE_LEN)
            .contains(&bytes.len())
    {
        return Some((Format::ArtStudio, Confidence::Probable));
    }

    // Last, and only ever `Probable`. A SCR is a raw dump of Spectrum screen
    // memory: no header, no magic, no structure a decoder could check. Every
    // 6,912-byte file is a valid one, so the length is the whole of the
    // evidence — hence the position in this function and the confidence it
    // returns. Promoting this to `Certain` would be a lie about what was
    // measured, and would make every other rule's `Certain` worth less.
    if bytes.len() == format198x_sinclair_zx_spectrum_scr::FILE_LEN {
        return Some((Format::Scr, Confidence::Probable));
    }

    None
}

/// Whether these bytes open an EA-IFF-85 `FORM` whose type is `ILBM`.
///
/// Spelled out here rather than taken from the ILBM crate, which exports no
/// constant for either four-byte tag — the only one of the five that does not.
/// Calling `decode` instead would parse the whole image to answer a question
/// about its first twelve bytes.
fn is_ilbm(bytes: &[u8]) -> bool {
    bytes.first_chunk::<4>() == Some(b"FORM")
        && bytes
            .get(8..12)
            .is_some_and(|form_type| form_type == b"ILBM")
}

/// The C64 load address in the first two bytes, little-endian, if there are two.
fn load_address(bytes: &[u8]) -> Option<u16> {
    bytes.first_chunk::<2>().map(|two| u16::from_le_bytes(*two))
}

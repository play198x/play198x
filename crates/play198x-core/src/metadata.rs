//! What an interface shows about a work: title, dimensions, sample names,
//! duration.
//!
//! Everything here is derived from something already decoded, so nothing in
//! this module reads bytes or holds a decoder. It exists to answer the
//! question a file browser, a thumbnailer or a player's info panel asks —
//! *what is this?* — in one shape per kind of work.
//!
//! # Sample names are content
//!
//! A ProTracker module's 31 sample slots each carry a 22-byte name field, and
//! a slot that holds no sample still has one. Authors used them: greetings,
//! credits, jokes, tracklists and whole paragraphs, laid out down the empty
//! half of the sample list where a tracker's own display shows them and
//! nothing overwrites them. So [`ModuleMeta::sample_names`] reports **all 31
//! slots, in slot order, empty ones included**. Listing only the slots a song
//! plays would drop the messages entirely, and quietly.
//!
//! The names, and the title with them, are ISO-8859-1: that is what Amiga text
//! is. [`Sample::name`] and [`Module::title`] decode it, which is why this
//! module calls them rather than reading `name_bytes` itself — a UTF-8 reading
//! of a name carrying an accent or a box-drawing byte comes back mangled or
//! empty.

use crate::decode::Image;
use crate::engine::{Engine, Timing};
use crate::probe::Format;
use format198x_commodore_amiga_mod::{Module, Sample};

/// What is known about one work, whichever kind it turned out to be.
///
/// Deliberately **not** `#[non_exhaustive]`, unlike the structs it holds and
/// unlike [`Format`]. A work here is something you look at or something you
/// listen to; if a third kind of work ever arrives, every interface that shows
/// one needs to decide what to do about it, and a wildcard arm each consumer
/// was forced to write years earlier is precisely how that decision gets
/// skipped. Adding a field to one of the structs is routine and stays
/// non-breaking; adding a kind of work is not routine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Metadata {
    /// A picture.
    Image(ImageMeta),
    /// A tune.
    Module(ModuleMeta),
    /// A code-driven tune: an `.ay` file. This variant, and [`AyMeta`]
    /// itself, are always present — only `ay_meta`, the sole constructor for
    /// `AyMeta` (which is `#[non_exhaustive]`, so nothing outside this
    /// module can build one by struct literal), is behind the `ay` feature,
    /// because it needs an `AyFile`. A build without `ay` therefore has no
    /// way to *produce* a `Metadata::Ay` — the benefit of keeping this
    /// variant ungated is not that such a build can receive one, but that
    /// every `match` on `Metadata`, everywhere, is forced to name this arm
    /// rather than getting to skip it because the feature that would fill
    /// it in happened to be off.
    Ay(AyMeta),
}

/// What a ProTracker module says about itself.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ModuleMeta {
    /// The 20-byte title field, Latin-1, trimmed at its first NUL.
    pub title: String,
    /// The four-byte magic as text — `M.K.`, `FLT4`, and the rest.
    pub format_tag: String,
    /// Voices the magic names: 4 for the modules this crate decodes.
    pub channels: u8,
    /// Patterns the file physically holds, which is not the same as the
    /// number the order table names: a module can carry patterns no position
    /// ever plays.
    pub patterns: usize,
    /// Order-table entries the song plays — its `song_length` prefix, not the
    /// format's fixed 128.
    pub orders: usize,
    /// One name per sample slot, in slot order: **always 31 entries**,
    /// including the slots that hold no sample. See the module documentation
    /// for why the empty ones are the interesting ones.
    pub sample_names: Vec<String>,
    /// How long one pass lasts, and whether the song comes back on itself.
    pub timing: Timing,
}

/// What a decoded picture says about itself, plus where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ImageMeta {
    /// What the bytes were identified as.
    pub format: Format,
    /// Width in mode pixels.
    pub width: u32,
    /// Height in mode pixels.
    pub height: u32,
    /// The colours the picture was drawn from, in hardware index order.
    pub palette: Vec<(u8, u8, u8)>,
    /// The container path the bytes came from — a
    /// [`Entry::path`](crate::container::Entry::path), or a plain file's own
    /// name. Supplied by the caller because a picture does not know where it
    /// was read from.
    pub source: String,
}

/// What an `.ay` file says about itself.
///
/// The ZXAY/EMUL header carries [`Self::author`] (`PAuthor`) and
/// [`Self::misc`] (`PMisc`) and nothing else — no title field, because the
/// format was built for tune collections where a *song* has a name and the
/// file holding them does not. [`Self::title`] and [`Self::length_frames`]
/// are read from the file's first song instead: a browser asking "what is
/// this?" needs one name to show, and the first song is the one a player
/// opens by default.
///
/// Reading song 0 is safe for any file `ay_meta` actually sees: `.ay`
/// stores a *last-song index* (`NumOfSongs`), so a stored `0` means one song,
/// not none, and `player::ay::format::parse` always returns at least one.
/// But `AyFile`'s fields are public, so nothing stops a caller building one
/// by hand with an empty `songs` — `ay_meta` reads song 0 defensively
/// rather than indexing it, and this struct says what that empty case
/// produces rather than leaving it to guesswork.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct AySong {
    /// The song's name from its entry in the file's song table.
    pub name: String,
    /// `SongLength` in 50Hz frames — how long a player plays it before
    /// fading. Frames, not milliseconds, because that is what the file says;
    /// an interface dividing by 50 is doing the conversion knowingly.
    pub length_frames: u16,
    /// `FadeLength` in 50Hz frames, following [`Self::length_frames`].
    pub fade_frames: u16,
}

/// What a `.ay` file says about itself.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct AyMeta {
    /// The first song's name, standing in for a title the format has no
    /// file-level field for. Empty when the file carries no songs.
    pub title: String,
    /// `PAuthor`, Latin-1, from the file header.
    pub author: String,
    /// `PMisc`, Latin-1, from the file header.
    pub misc: String,
    /// Every song, in file order — the full tune list a player would offer,
    /// not just the one [`Self::title`] is drawn from.
    ///
    /// Each carries its own length, rather than the file carrying one: an
    /// interface that lets a visitor choose a song needs that song's
    /// duration, and a single file-level figure is song 0's answer given to
    /// every question.
    pub songs: Vec<AySong>,
}

/// Describe a module.
///
/// # Cost
///
/// [`Timing`] needs a walk of the whole sequence, so this builds a throwaway
/// [`Engine`] to do one. **A caller that already has an engine should read
/// [`Engine::timing`] directly** and fill the field from that, rather than
/// paying for a second walk and a clone of the module.
///
/// `sample_rate` is the rate that engine is built at. It does not change any
/// number reported here — a tick is `2500 / tempo` milliseconds whatever the
/// output rate is — and is taken only so the walk happens under the same
/// engine a caller would play with.
#[must_use]
pub fn module_meta(module: &Module, sample_rate: u32) -> ModuleMeta {
    ModuleMeta {
        title: module.title(),
        format_tag: latin1(&module.magic),
        // From the module, which reads its own magic once. Re-parsing the
        // magic here would be a second copy of that rule, free to drift.
        channels: module.channels(),
        patterns: module.patterns.len(),
        orders: module.orders().len(),
        // `module.samples` is an array of 31, so this is every slot by
        // construction. Nothing filters it, and nothing should: see the module
        // documentation.
        sample_names: module.samples.iter().map(Sample::name).collect(),
        timing: Engine::new(module.clone(), sample_rate).timing(),
    }
}

/// Describe a decoded picture that was read from `source`.
#[must_use]
pub fn image_meta(image: &Image, source: &str) -> ImageMeta {
    ImageMeta {
        format: image.format,
        width: image.width,
        height: image.height,
        // From the decode that produced the pixels — the spec's table for the
        // fixed-palette machines, the file's own CMAP for an ILBM. Never a
        // table written down here, and never derived from the pixels, which
        // would silently lose every colour the picture did not use.
        palette: image.palette.clone(),
        source: source.to_owned(),
    }
}

/// Describe an `.ay` file.
///
/// Behind the `ay` feature and not the struct it builds: this is the one
/// piece of `.ay` metadata that needs an [`AyFile`](crate::player::ay::format::AyFile)
/// to read, and that type only exists when `ay` is enabled. [`AyMeta`] and
/// [`Metadata::Ay`] carry no such dependency, so a build without `ay` can
/// still name and hold the shape — it just cannot fill one in from a real
/// file itself.
#[cfg(feature = "ay")]
#[must_use]
pub fn ay_meta(file: &crate::player::ay::format::AyFile) -> AyMeta {
    let first = file.songs.first();
    AyMeta {
        title: first.map(|song| song.name.clone()).unwrap_or_default(),
        author: file.author.clone(),
        misc: file.misc.clone(),
        songs: file
            .songs
            .iter()
            .map(|song| AySong {
                name: song.name.clone(),
                length_frames: song.length_frames,
                fade_frames: song.fade_frames,
            })
            .collect(),
    }
}

/// Read bytes as ISO-8859-1.
///
/// Used for the format tag, which every real file writes in ASCII — but
/// `Module`'s fields are public, so a caller can hand over four bytes that are
/// not, and the answer to that must be text rather than a panic or a
/// replacement character. Latin-1 maps all 256 byte values, so there is
/// nothing to fail on.
fn latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| char::from(b)).collect()
}

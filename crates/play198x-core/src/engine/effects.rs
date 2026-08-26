//! ProTracker's three effect dispatch tables.
//!
//! The replayer has three of them, not one, and which runs depends on whether
//! the tick fetches a new row and whether that row carries a note:
//!
//! | Table | When it runs | Handles |
//! |---|---|---|
//! | `prefx_tab` | note tick, **before** the period is set | `3` `5` tone portamento, `9` sample offset |
//! | `morefx_tab` | note tick after note setup, and on a row with no new note | `9` `B` `C` `D` `E` `F` |
//! | `fx_tab` | every tick that does **not** fetch a row | `0` `1` `2` `3` `4` `5` `6` `7` `A` `E` |
//!
//! Effect `9` appears in *two* of them, which a single `match` cannot express:
//! a sample offset must act whether or not the row carries a new note. That is
//! the structural reason for three functions rather than one.
//!
//! The consequence of `mt_checkfx` not running on the note tick is that **a
//! per-tick effect applies `speed - 1` times per row, not `speed`** — five
//! times at the default speed 6. A single table, or a loop that runs `speed`
//! times, is a silent 20% error in every per-tick effect.
//!
//! Everything here is read from `protracker-23b-playroutine.asm` (Frank
//! Wille's Protracker V2.3B playroutine, public domain), with the line numbers
//! quoted per function. Where the widely-cited community specification
//! disagrees — it does, about the vibrato rate, by 20% — the replayer wins.
//! See `reference/by-topic/music-formats/protracker-playback-reference.md`.

use super::{CHANNELS, MAX_VOLUME, ROWS_PER_PATTERN, Seq, Voice};

/// The number of notes ProTracker's period tables hold, C-1 to B-3.
pub(super) const NOTES: usize = 36;

/// The lowest period `1xx` will slide to (`do_porta_up`, line 2013).
const MIN_PERIOD: i32 = 113;

/// The highest period `2xx` will slide to (`do_porta_down`, line 2036).
const MAX_PERIOD: i32 = 856;

/// The boundary between `Fxy` meaning speed and meaning tempo.
///
/// `mt_setspeed` (line 2338) is `cmp.b #$20,d4 / bhs`, so `$00..$1F` set the
/// speed and `$20` upwards set the tempo. The community specification's "z <
/// 32" agrees; the distilled reference's "xy <= 32" does not, and the replayer
/// is the authority.
const SPEED_TEMPO_BOUNDARY: u8 = 0x20;

/// ProTracker's period tables, one per finetune value, in the replayer's own
/// order: `0, +1..+7, -8..-1` (`mt_PerFineTune`, line 2846). Index a table by
/// note, `0..36`.
///
/// The negative-finetune tables start a semitone lower than the others, which
/// is why the note search runs over the *untuned* table and only tone
/// portamento corrects for it.
// Laid out an octave to a line, as the replayer's own `dc.w` blocks are: the
// shape is how a reader checks a value against the source.
#[rustfmt::skip]
pub(super) const PERIOD_TABLE: [[u16; NOTES]; 16] = [
    // finetune 0
    [
        856, 808, 762, 720, 678, 640, 604, 570, 538, 508, 480, 453,
        428, 404, 381, 360, 339, 320, 302, 285, 269, 254, 240, 226,
        214, 202, 190, 180, 170, 160, 151, 143, 135, 127, 120, 113,
    ],
    // finetune +1
    [
        850, 802, 757, 715, 674, 637, 601, 567, 535, 505, 477, 450,
        425, 401, 379, 357, 337, 318, 300, 284, 268, 253, 239, 225,
        213, 201, 189, 179, 169, 159, 150, 142, 134, 126, 119, 113,
    ],
    // finetune +2
    [
        844, 796, 752, 709, 670, 632, 597, 563, 532, 502, 474, 447,
        422, 398, 376, 355, 335, 316, 298, 282, 266, 251, 237, 224,
        211, 199, 188, 177, 167, 158, 149, 141, 133, 125, 118, 112,
    ],
    // finetune +3
    [
        838, 791, 746, 704, 665, 628, 592, 559, 528, 498, 470, 444,
        419, 395, 373, 352, 332, 314, 296, 280, 264, 249, 235, 222,
        209, 198, 187, 176, 166, 157, 148, 140, 132, 125, 118, 111,
    ],
    // finetune +4
    [
        832, 785, 741, 699, 660, 623, 588, 555, 524, 495, 467, 441,
        416, 392, 370, 350, 330, 312, 294, 278, 262, 247, 233, 220,
        208, 196, 185, 175, 165, 156, 147, 139, 131, 124, 117, 110,
    ],
    // finetune +5
    [
        826, 779, 736, 694, 655, 619, 584, 551, 520, 491, 463, 437,
        413, 390, 368, 347, 328, 309, 292, 276, 260, 245, 232, 219,
        206, 195, 184, 174, 164, 155, 146, 138, 130, 123, 116, 109,
    ],
    // finetune +6
    [
        820, 774, 730, 689, 651, 614, 580, 547, 516, 487, 460, 434,
        410, 387, 365, 345, 325, 307, 290, 274, 258, 244, 230, 217,
        205, 193, 183, 172, 163, 154, 145, 137, 129, 122, 115, 109,
    ],
    // finetune +7
    [
        814, 768, 725, 684, 646, 610, 575, 543, 513, 484, 457, 431,
        407, 384, 363, 342, 323, 305, 288, 272, 256, 242, 228, 216,
        204, 192, 181, 171, 161, 152, 144, 136, 128, 121, 114, 108,
    ],
    // finetune -8
    [
        907, 856, 808, 762, 720, 678, 640, 604, 570, 538, 508, 480,
        453, 428, 404, 381, 360, 339, 320, 302, 285, 269, 254, 240,
        226, 214, 202, 190, 180, 170, 160, 151, 143, 135, 127, 120,
    ],
    // finetune -7
    [
        900, 850, 802, 757, 715, 675, 636, 601, 567, 535, 505, 477,
        450, 425, 401, 379, 357, 337, 318, 300, 284, 268, 253, 238,
        225, 212, 200, 189, 179, 169, 159, 150, 142, 134, 126, 119,
    ],
    // finetune -6
    [
        894, 844, 796, 752, 709, 670, 632, 597, 563, 532, 502, 474,
        447, 422, 398, 376, 355, 335, 316, 298, 282, 266, 251, 237,
        223, 211, 199, 188, 177, 167, 158, 149, 141, 133, 125, 118,
    ],
    // finetune -5
    [
        887, 838, 791, 746, 704, 665, 628, 592, 559, 528, 498, 470,
        444, 419, 395, 373, 352, 332, 314, 296, 280, 264, 249, 235,
        222, 209, 198, 187, 176, 166, 157, 148, 140, 132, 125, 118,
    ],
    // finetune -4
    [
        881, 832, 785, 741, 699, 660, 623, 588, 555, 524, 494, 467,
        441, 416, 392, 370, 350, 330, 312, 294, 278, 262, 247, 233,
        220, 208, 196, 185, 175, 165, 156, 147, 139, 131, 123, 117,
    ],
    // finetune -3
    [
        875, 826, 779, 736, 694, 655, 619, 584, 551, 520, 491, 463,
        437, 413, 390, 368, 347, 328, 309, 292, 276, 260, 245, 232,
        219, 206, 195, 184, 174, 164, 155, 146, 138, 130, 123, 116,
    ],
    // finetune -2
    [
        868, 820, 774, 730, 689, 651, 614, 580, 547, 516, 487, 460,
        434, 410, 387, 365, 345, 325, 307, 290, 274, 258, 244, 230,
        217, 205, 193, 183, 172, 163, 154, 145, 137, 129, 122, 115,
    ],
    // finetune -1
    [
        862, 814, 768, 725, 684, 646, 610, 575, 543, 513, 484, 457,
        431, 407, 384, 363, 342, 323, 305, 288, 272, 256, 242, 228,
        216, 203, 192, 181, 171, 161, 152, 144, 136, 128, 121, 114,
    ],
];

/// A quarter-to-half sine, the 32 bytes every vibrato and tremolo waveform
/// value is scaled from (`mt_VibratoSineTable`, line 2645).
const SINE: [u8; 32] = [
    0, 24, 49, 74, 97, 120, 141, 161, 180, 197, 212, 224, 235, 244, 250, 253, 255, 253, 250, 244,
    235, 224, 212, 197, 180, 161, 141, 120, 97, 74, 49, 24,
];

/// The vibrato or tremolo offset for a waveform, position and amplitude.
///
/// The replayer reads a precomputed 1024-byte table per waveform, indexed at
/// `64 * amplitude + (position & 63)`. All three tables are exactly reproduced
/// by scaling a raw waveform value by `amplitude / 128` with the truncating
/// division a 68000 `divs` does — verified against the replayer's own bytes,
/// all 1,024 entries of all three tables, zero mismatches.
///
/// The sawtooth is *not* the sine's negate-the-second-half shape: it ramps up
/// across `0..32`, jumps to its negative extreme at 32 and ramps up again, so
/// it needs its own expression rather than a shared sign flip.
/// (`mt_VibratoSineTable` 2645, `mt_VibratoSawTable` 2712,
/// `mt_VibratoRectTable` 2778; selected by `n_vibratoctrl & 3` at line 2133.)
fn waveform(ctrl: u8, position: u8, amplitude: u8) -> i32 {
    let at = i32::from(position & 63);
    let quarter = (at & 31) as usize;
    let raw = match ctrl & 3 {
        // Sine: the quarter table, mirrored and then negated.
        0 => {
            let value = i32::from(SINE[quarter]);
            if at >= 32 { -value } else { value }
        }
        // Sawtooth.
        1 => {
            if at < 32 {
                at * 8
            } else {
                at * 8 - 511
            }
        }
        // Rectangle: full scale, sign by half.
        _ => {
            if at >= 32 {
                -255
            } else {
                255
            }
        }
    };
    raw * i32::from(amplitude) / 128
}

/// The index of the first note whose untuned period is at or below `period`.
///
/// `set_period` (line 1852) walks `mt_PeriodTable` with `dbhs`, stopping at
/// the first entry the note is not above; a note below every entry leaves the
/// index at the last note rather than running off the table.
fn nearest_note(table: &[u16; NOTES], period: u16) -> usize {
    for (index, entry) in table.iter().enumerate() {
        if period >= *entry {
            return index;
        }
    }
    NOTES - 1
}

impl Seq<'_> {
    /// `mt_playvoice` (line 1637): act on one channel's cell at the note tick.
    ///
    /// The sample number is taken first and acts whether or not there is a
    /// note — a sample number alone sets the volume without retriggering.
    /// Then `prefx_tab` runs, then the period is set, then `morefx_tab`.
    pub(super) fn play_voice(&mut self, channel: usize, note: super::Note) {
        let rate = self.sample_rate;
        {
            let voice = &mut self.state.voices[channel];
            // `tst.l (a2)` at line 1667: an entirely empty previous cell means
            // nothing has written AUDPER since, so the stored period is
            // re-asserted. This is what ends an arpeggio or a vibrato.
            if voice.cell_empty {
                let period = voice.period;
                voice.set_audper(period, rate);
            }
            voice.note_period = note.period;
            voice.effect = note.effect & 0x0F;
            voice.param = note.param;
            voice.cell_empty =
                note.sample == 0 && note.period == 0 && note.effect == 0 && note.param == 0;
        }

        if note.sample != 0 {
            select_sample(&mut self.state.voices[channel], note.sample, self.module);
        }

        // No note: straight to `morefx_tab` (`tst.w d6 / beq checkmorefx`,
        // line 1789). This is the path a bare `C40` or `900` takes.
        if note.period == 0 {
            self.morefx(channel);
            return;
        }

        let (effect, param) = (
            self.state.voices[channel].effect,
            self.state.voices[channel].param,
        );

        // `E5x` set finetune is checked ahead of the table (line 1792): it
        // changes which period table the note is looked up in, so it cannot
        // wait until `morefx_tab`.
        if effect == 0x0E && param >> 4 == 0x05 {
            self.state.voices[channel].finetune = usize::from(param & 0x0F);
            self.set_period(channel, note.period);
            return;
        }

        // prefx_tab (line 1797).
        match effect {
            // Tone portamento retargets instead of retriggering, and returns
            // without running `morefx_tab` — the replayer `jmp`s here rather
            // than calling, and `set_toneporta` ends in `rts`. Both effects'
            // `morefx_tab` entries are no-ops, so nothing is lost by it.
            0x03 | 0x05 => self.set_toneporta(channel, note.period),
            // `set_sampleoffset` (line 1830) is `bsr mt_sampleoffset / bra
            // set_period`, and `set_period` falls through to `morefx_tab`,
            // whose `$9` entry is `mt_sampleoffset` again — so a row with both
            // a note and `9xy` really does apply the offset twice. The sample
            // has already restarted by the second one, so what it changes is
            // where a *later* retrigger with no new sample number begins.
            // Reproduced rather than tidied away: it is what the replayer does.
            0x09 => {
                self.sample_offset(channel);
                self.set_period(channel, note.period);
            }
            _ => self.set_period(channel, note.period),
        }
    }

    /// `set_period` (line 1852): resolve the note to a period, retrigger, then
    /// fall through to `morefx_tab`.
    fn set_period(&mut self, channel: usize, note_period: u16) {
        let rate = self.sample_rate;
        {
            let voice = &mut self.state.voices[channel];
            let index = nearest_note(&PERIOD_TABLE[0], note_period);
            voice.note_index = index;
            voice.period = PERIOD_TABLE[voice.finetune][index];

            // `EDx` note delay: the period is resolved but the sample does not
            // start until tick x (line 1868).
            if !(voice.effect == 0x0E && voice.param >> 4 == 0x0D) {
                if voice.vib_ctrl & 4 == 0 {
                    voice.vib_pos = 0;
                }
                if voice.trem_ctrl & 4 == 0 {
                    voice.trem_pos = 0;
                }
                voice.retrigger();
                let period = voice.period;
                voice.set_audper(period, rate);
            }
        }
        self.morefx(channel);
    }

    /// `set_toneporta` (line 1919): aim the slide at the row's note instead of
    /// playing it.
    fn set_toneporta(&mut self, channel: usize, note_period: u16) {
        let voice = &mut self.state.voices[channel];
        let table = &PERIOD_TABLE[voice.finetune];
        let mut index = nearest_note(table, note_period);
        // A negative finetune's table is shifted a semitone, so the search
        // lands one note high; the replayer steps back (line 1927).
        if voice.finetune >= 8 && index != 0 {
            index -= 1;
        }
        voice.note_index = index;
        let target = table[index];
        // Already there: no target, so the per-tick half does nothing.
        voice.wanted_period = if target == voice.period { 0 } else { target };
    }

    /// `morefx_tab` (line 1899): the note tick's second dispatch, and the only
    /// dispatch on a row with no new note.
    pub(super) fn morefx(&mut self, channel: usize) {
        let (effect, param) = (
            self.state.voices[channel].effect,
            self.state.voices[channel].param,
        );
        match effect {
            0x09 => self.sample_offset(channel),
            0x0B => self.position_jump(param),
            0x0C => self.set_volume(channel, param),
            0x0D => self.pattern_break(param),
            0x0E => self.e_command(channel),
            0x0F => self.set_speed(param),
            // `mt_pernop` (line 1628): re-assert the stored period.
            _ => self.per_nop(channel),
        }
    }

    /// `fx_tab` (line 1609): every tick that does not fetch a row.
    pub(super) fn fx(&mut self, channel: usize) {
        let (effect, param) = (
            self.state.voices[channel].effect,
            self.state.voices[channel].param,
        );
        // `d4 = n_cmd & $0fff / beq mt_pernop` (line 1597): effect 0 with a
        // zero parameter is not an arpeggio, it is nothing at all.
        if effect == 0 && param == 0 {
            self.per_nop(channel);
            return;
        }
        match effect {
            0x00 => self.arpeggio(channel, param),
            0x01 => self.porta_up(channel, u16::from(param)),
            0x02 => self.porta_down(channel, u16::from(param)),
            0x03 => self.tone_porta(channel, param),
            0x04 => self.vibrato(channel, param),
            0x05 => {
                self.tone_porta_nc(channel);
                self.volume_slide(channel, param);
            }
            0x06 => {
                self.vibrato_nc(channel);
                self.volume_slide(channel, param);
            }
            0x07 => self.tremolo(channel, param),
            0x0A => self.volume_slide(channel, param),
            0x0E => self.e_command(channel),
            // `mt_nop` (line 1630): leaves AUDPER alone rather than
            // re-asserting it, which is not the same thing.
            _ => {}
        }
    }

    /// `mt_e_cmds` (line 2362). Reached from both `morefx_tab` and `fx_tab`;
    /// the sub-effects that act once a row test the tick counter themselves,
    /// exactly as the replayer does.
    fn e_command(&mut self, channel: usize) {
        let param = self.state.voices[channel].param;
        let x = param & 0x0F;
        let note_tick = self.state.tick == 0;
        match param >> 4 {
            // `E0x` toggles the Amiga's LED low-pass filter. Nothing here
            // models that filter, so nothing here can honour it; silently
            // doing nothing is the only answer that is not a lie.
            0x0 => {}
            0x1 => {
                if note_tick {
                    self.do_porta_up(channel, u16::from(x));
                }
            }
            0x2 => {
                if note_tick {
                    self.do_porta_down(channel, u16::from(x));
                }
            }
            0x3 => self.state.voices[channel].glissando = x != 0,
            0x4 => self.state.voices[channel].vib_ctrl = x,
            0x5 => self.state.voices[channel].finetune = usize::from(x),
            0x6 => self.pattern_loop(channel, x),
            0x7 => self.state.voices[channel].trem_ctrl = x,
            // `E8x` stores a value for a host program to read back. There is
            // no host program here, and it makes no sound.
            0x8 => {}
            0x9 => self.retrigger_note(channel, x),
            0xA => {
                if note_tick {
                    self.fine_volume_up(channel, x);
                }
            }
            0xB => {
                if note_tick {
                    self.fine_volume_down(channel, x);
                }
            }
            0xC => self.note_cut(channel, x),
            0xD => self.note_delay(channel, x),
            0xE => self.pattern_delay(x),
            // `EFx` funk repeat inverts bytes of the sample *in place* as it
            // plays (`mt_updatefunk`, line 2604). This engine holds the module
            // immutably and shares it with metadata and duration, so the one
            // effect that rewrites its own instrument is left out rather than
            // implemented by copying every sample on load.
            _ => {}
        }
    }

    /// `mt_pernop` (line 1628): write the stored period to AUDPER.
    fn per_nop(&mut self, channel: usize) {
        let rate = self.sample_rate;
        let voice = &mut self.state.voices[channel];
        let period = voice.period;
        voice.set_audper(period, rate);
    }

    /// `mt_arpeggio` (line 1971): step base, `x`, `y` with the tick counter.
    ///
    /// The replayer's `arptab` is 32 bytes of `0, 1, -1` repeating, which is
    /// exactly `tick mod 3` — and the tick can never reach 32, because `Fxy`
    /// only sets a speed below `$20`.
    fn arpeggio(&mut self, channel: usize, param: u8) {
        let semitones = match self.state.tick % 3 {
            0 => {
                self.per_nop(channel);
                return;
            }
            1 => usize::from(param >> 4),
            _ => usize::from(param & 0x0F),
        };
        let rate = self.sample_rate;
        let voice = &mut self.state.voices[channel];
        let index = voice.note_index + semitones;
        // Past the top of the table the replayer returns without writing
        // AUDPER at all, leaving whatever the last write put there.
        if index < NOTES {
            let period = PERIOD_TABLE[voice.finetune][index];
            voice.set_audper(period, rate);
        }
    }

    /// `mt_portaup` (line 2004): subtract from the period every tick.
    fn porta_up(&mut self, channel: usize, amount: u16) {
        self.do_porta_up(channel, amount);
    }

    /// `do_porta_up` (line 2009), shared with `E1x` fine portamento.
    fn do_porta_up(&mut self, channel: usize, amount: u16) {
        let rate = self.sample_rate;
        let voice = &mut self.state.voices[channel];
        let period = (i32::from(voice.period) - i32::from(amount)).max(MIN_PERIOD);
        voice.period = period as u16;
        voice.set_audper(period as u16, rate);
    }

    /// `mt_portadown` (line 2031).
    fn porta_down(&mut self, channel: usize, amount: u16) {
        self.do_porta_down(channel, amount);
    }

    /// `do_porta_down` (line 2036), shared with `E2x` fine portamento.
    fn do_porta_down(&mut self, channel: usize, amount: u16) {
        let rate = self.sample_rate;
        let voice = &mut self.state.voices[channel];
        let period = (i32::from(voice.period) + i32::from(amount)).min(MAX_PERIOD);
        voice.period = period as u16;
        voice.set_audper(period as u16, rate);
    }

    /// `mt_toneporta` (line 2048): a non-zero parameter sets the speed.
    fn tone_porta(&mut self, channel: usize, param: u8) {
        if param != 0 {
            self.state.voices[channel].tone_speed = param;
            // `move.b d7,n_cmdlo`: the replayer clears the parameter so the
            // rest of the row cannot set the speed twice.
            self.state.voices[channel].param = 0;
        }
        self.tone_porta_nc(channel);
    }

    /// `mt_toneporta_nc` (line 2053): one step towards the target.
    fn tone_porta_nc(&mut self, channel: usize) {
        let rate = self.sample_rate;
        let voice = &mut self.state.voices[channel];
        let target = i32::from(voice.wanted_period);
        if target == 0 {
            return;
        }
        let speed = i32::from(voice.tone_speed);
        let mut period = i32::from(voice.period);
        if period < target {
            period += speed;
            if period >= target {
                period = target;
                voice.wanted_period = 0;
            }
        } else {
            period -= speed;
            if period <= target {
                period = target;
                voice.wanted_period = 0;
            }
        }
        voice.period = period as u16;

        // Glissando (`E3x`) snaps what Paula hears to a semitone, but leaves
        // the stored period where the slide actually got to (line 2078): the
        // slide keeps its sub-semitone progress while the pitch steps.
        let sounding = if voice.glissando {
            let index = nearest_note(&PERIOD_TABLE[voice.finetune], period as u16);
            voice.note_index = index;
            PERIOD_TABLE[voice.finetune][index]
        } else {
            period as u16
        };
        voice.set_audper(sounding, rate);
    }

    /// `mt_vibrato` (line 2109): `x` is speed, `y` is amplitude, and a zero
    /// nibble keeps the previous value — which is what makes a bare `400` on
    /// the next row continue the sweep.
    fn vibrato(&mut self, channel: usize, param: u8) {
        {
            let voice = &mut self.state.voices[channel];
            if param & 0x0F != 0 {
                voice.vib_amp = param & 0x0F;
            }
            if param >> 4 != 0 {
                voice.vib_speed = param >> 4;
            }
        }
        self.vibrato_nc(channel);
    }

    /// `mt_vibrato_nc` (line 2119). The delta shifts the *playing* period only
    /// and is never stored back (line 2153) — a player that stores it
    /// integrates the sweep and the note sails away instead of wobbling.
    fn vibrato_nc(&mut self, channel: usize) {
        let rate = self.sample_rate;
        let voice = &mut self.state.voices[channel];
        let delta = waveform(voice.vib_ctrl, voice.vib_pos, voice.vib_amp);
        let sounding = (i32::from(voice.period) + delta).max(1);
        voice.set_audper(sounding.min(i32::from(u16::MAX)) as u16, rate);
        // `add.b d4,n_vibratopos`: a byte, so it wraps at 256 rather than 64,
        // and the table index masks to 63 separately.
        voice.vib_pos = voice.vib_pos.wrapping_add(voice.vib_speed);
    }

    /// `mt_tremolo` (line 2172): vibrato's machinery applied to the volume.
    fn tremolo(&mut self, channel: usize, param: u8) {
        let rate = self.sample_rate;
        let voice = &mut self.state.voices[channel];
        if param & 0x0F != 0 {
            voice.trem_amp = param & 0x0F;
        }
        if param >> 4 != 0 {
            voice.trem_speed = param >> 4;
        }
        let delta = waveform(voice.trem_ctrl, voice.trem_pos, voice.trem_amp);
        // The replayer does this in a byte and tests the sign; the offset can
        // never exceed +/-29 (255 * 15 / 128), so a signed 16-bit clamp is the
        // same arithmetic without the wrap.
        let level = (i32::from(voice.volume) + delta).clamp(0, i32::from(MAX_VOLUME));
        voice.play_volume = level as u8;
        // Tremolo re-asserts the period as well (line 2229), which matters on
        // a row where an arpeggio ran on an earlier tick.
        let period = voice.period;
        voice.set_audper(period, rate);
        voice.trem_pos = voice.trem_pos.wrapping_add(voice.trem_speed);
    }

    /// `mt_volumeslide` (line 2246): up by the high nibble, else down by the
    /// low one. A non-zero high nibble wins; `A11` slides up.
    fn volume_slide(&mut self, channel: usize, param: u8) {
        let voice = &mut self.state.voices[channel];
        let level = if param >> 4 != 0 {
            i32::from(voice.volume) + i32::from(param >> 4)
        } else {
            i32::from(voice.volume) - i32::from(param & 0x0F)
        };
        let level = level.clamp(0, i32::from(MAX_VOLUME)) as u8;
        voice.volume = level;
        voice.play_volume = level;
    }

    /// `mt_volfineup` (line 2537): `EAx`, once per row.
    fn fine_volume_up(&mut self, channel: usize, amount: u8) {
        let voice = &mut self.state.voices[channel];
        let level = (i32::from(voice.volume) + i32::from(amount)).min(i32::from(MAX_VOLUME)) as u8;
        voice.volume = level;
        voice.play_volume = level;
    }

    /// `mt_volfinedn` (line 2549): `EBx`, once per row.
    fn fine_volume_down(&mut self, channel: usize, amount: u8) {
        let voice = &mut self.state.voices[channel];
        let level = (i32::from(voice.volume) - i32::from(amount)).max(0) as u8;
        voice.volume = level;
        voice.play_volume = level;
    }

    /// `mt_volchange` (line 2295): `Cxy`, clamped to full volume.
    fn set_volume(&mut self, channel: usize, param: u8) {
        let voice = &mut self.state.voices[channel];
        let level = param.min(MAX_VOLUME);
        voice.volume = level;
        voice.play_volume = level;
    }

    /// `mt_sampleoffset` (line 1812): start the sample `param * 256` bytes in.
    ///
    /// A parameter of `00` reuses the channel's remembered offset, which is
    /// what makes `900` useful rather than a no-op. An offset past the end
    /// leaves one word to play rather than reading off the end of the sample.
    fn sample_offset(&mut self, channel: usize) {
        let voice = &mut self.state.voices[channel];
        let param = if voice.param == 0 {
            voice.sample_offset
        } else {
            voice.sample_offset = voice.param;
            voice.param
        };
        let offset = usize::from(param) * 256;
        if offset >= voice.length {
            voice.length = 2;
        } else {
            voice.length -= offset;
            voice.start += offset;
        }
    }

    /// `mt_retrignote` (line 2508): `E9x` restarts the sample every `x` ticks.
    fn retrigger_note(&mut self, channel: usize, count: u8) {
        if count == 0 {
            return;
        }
        let voice = &mut self.state.voices[channel];
        if self.state.tick == 0 {
            voice.retrig_count = count;
            // A row that carries a note has already triggered it; retriggering
            // again on the same tick would double it.
            if voice.note_period != 0 {
                return;
            }
        } else {
            voice.retrig_count = voice.retrig_count.wrapping_sub(1);
            if voice.retrig_count != 0 {
                return;
            }
            voice.retrig_count = count;
        }
        voice.retrigger();
    }

    /// `mt_notecut` (line 2561): `ECx` zeroes the volume on tick `x`.
    fn note_cut(&mut self, channel: usize, at: u8) {
        if at != self.state.tick {
            return;
        }
        let voice = &mut self.state.voices[channel];
        voice.volume = 0;
        voice.play_volume = 0;
    }

    /// `mt_notedelay` (line 2572): `EDx` starts the row's note on tick `x`.
    fn note_delay(&mut self, channel: usize, at: u8) {
        if at != self.state.tick {
            return;
        }
        let rate = self.sample_rate;
        let voice = &mut self.state.voices[channel];
        // Only a row that actually carries a note is delayed; `ED2` alone
        // retriggers nothing.
        if voice.note_period == 0 {
            return;
        }
        let period = voice.period;
        voice.set_audper(period, rate);
        voice.retrigger();
    }

    /// `mt_patterndelay` (line 2588): `EEx` replays the row `x` more times.
    ///
    /// The extra rounds do not re-fetch the row, so nothing retriggers; only
    /// `fx_tab` runs, and it runs on the note tick too.
    fn pattern_delay(&mut self, count: u8) {
        if self.state.tick != 0 || self.state.patt_del_time2 != 0 {
            return;
        }
        self.state.patt_del_time = count.saturating_add(1);
    }

    /// `mt_jumploop` (line 2478): `E60` marks a loop start, `E6x` repeats back
    /// to it `x` times.
    fn pattern_loop(&mut self, channel: usize, count: u8) {
        if self.state.tick != 0 {
            return;
        }
        let row = self.state.row;
        let voice = &mut self.state.voices[channel];
        if count == 0 {
            voice.loop_row = row;
            return;
        }
        let remaining = voice.loop_count.wrapping_sub(1);
        if remaining == 0 {
            voice.loop_count = 0;
            return;
        }
        // A negative count is the "not started yet" state the replayer gets
        // from decrementing a zero byte.
        voice.loop_count = if remaining < 0 {
            count.min(i8::MAX as u8) as i8
        } else {
            remaining
        };
        self.state.pbreak_row = self.state.voices[channel].loop_row;
        self.state.pbreak_flag = true;
    }

    /// `mt_posjump` (line 2280): `Bxy` continues at order `xy`.
    ///
    /// The replayer sets the position to `xy - 1` and lets the ordinary
    /// end-of-row step add one back, which is why `B00` restarts the song.
    fn position_jump(&mut self, param: u8) {
        self.state.order = usize::from(param.wrapping_sub(1) & 0x7F);
        self.state.pbreak_row = 0;
        self.state.posjump_flag = true;
    }

    /// `mt_patternbrk` (line 2311): `Dxy` continues at row `xy` — read as
    /// decimal, because trackers showed row numbers in decimal.
    fn pattern_break(&mut self, param: u8) {
        let tens = usize::from(param >> 4);
        let row = if tens < 10 { tens * 10 } else { 0 } + usize::from(param & 0x0F);
        self.state.pbreak_row = if row < ROWS_PER_PATTERN { row } else { 0 };
        self.state.posjump_flag = true;
    }

    /// `mt_setspeed` (line 2338): below `$20` it is ticks per row, at or above
    /// it is beats per minute.
    ///
    /// **`F00` stops the module.** The replayer stores the zero speed and then
    /// branches to `_mt_end` (line 2347), which clears `mt_Enable` and resets
    /// the four channels — it does not carry on at some clamped speed, which
    /// is what a player that only guards the division does. The difference is
    /// audible on any module that ends with `F00`, and it is the difference
    /// between a length and the whole rest of the pattern.
    fn set_speed(&mut self, param: u8) {
        if param >= SPEED_TEMPO_BOUNDARY {
            self.state.tempo = u16::from(param);
            return;
        }
        self.state.speed = param;
        if param == 0 {
            self.state.stopped = true;
            // `resetch` on all four channels: `_mt_end` silences them rather
            // than leaving the last note ringing.
            for voice in &mut self.state.voices {
                voice.active = false;
            }
        }
    }

    /// Run `fx_tab` for every channel — one ordinary tick.
    pub(super) fn run_fx(&mut self) {
        for channel in 0..CHANNELS {
            self.fx(channel);
        }
    }
}

/// The sample-number half of `mt_playvoice` (line 1697).
///
/// Selects the slot, takes its volume and finetune, and points the voice at
/// the whole sample again — which is how a new sample number cancels an `9xx`
/// offset left over from an earlier row.
fn select_sample(voice: &mut Voice, number: u8, module: &super::Module) {
    let slot = usize::from(number) - 1;
    // A sample number past the 31 slots selects nothing. Files do this;
    // ProTracker reads whatever is at that offset, which is not a behaviour
    // worth reproducing.
    let Some(sample) = module.samples.get(slot) else {
        return;
    };
    voice.sample = Some(slot);
    voice.data_len = sample.data.len();
    voice.volume = sample.volume.min(MAX_VOLUME);
    voice.play_volume = voice.volume;
    voice.finetune = usize::from(sample.finetune_byte & 0x0F);

    let len = sample.data.len();
    // Clamped, because a header can point its loop past the end of its own
    // data and this crate does not get to panic about that. A loop clamped to
    // nothing becomes a one-shot: the sample plays through and stops.
    let loop_start = sample.loop_start().min(len);
    let loop_len = if sample.is_looped() {
        sample.loop_len().min(len - loop_start)
    } else {
        0
    };
    voice.loop_start = loop_start;
    voice.loop_len = loop_len;
    voice.start = 0;
    // The first pass is not always the loop. ProTracker programs Paula with
    // `repeat_start + repeat_length` words when the loop starts partway in,
    // and with the whole sample when it starts at zero — so a sample with a
    // short loop at offset zero still plays all the way through once before it
    // begins repeating (`set_len_start`, line 1774).
    voice.length = if loop_len > 0 && sample.repeat_start_words != 0 {
        (loop_start + loop_len).min(len)
    } else {
        len
    };
}

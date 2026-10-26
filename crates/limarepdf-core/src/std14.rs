//! Glyph widths of the standard PDF fonts, for text set in one of them when the
//! document does not give the widths, and for text LimarePDF adds in Helvetica; and the
//! characters of Symbol's and ZapfDingbats' built-in encodings, for reading their text.
//!
//! The tables are generated from Adobe's Core 14 AFM files by `tools/gen_std14.py`.
//! Modified: only the advance widths (WX) are kept, as tables indexed by code: of the
//! characters in the Windows-1252 set for Helvetica and Times, and of the codes of their
//! built-in encodings for Symbol and ZapfDingbats; 0 marks a code with no character. The
//! oblique and Courier fonts are left out: oblique widths are the upright ones, and
//! Courier is 600 throughout. For Symbol and ZapfDingbats each code's glyph name (N) is
//! kept too, as the character it names (by the Adobe Glyph List, and for ZapfDingbats the
//! ITC Zapf Dingbats Glyph List), in tables indexed by code; NUL marks a code with none.
//!
//! Helvetica.afm: Copyright (c) 1985, 1987, 1989, 1990, 1997 Adobe Systems Incorporated.  All Rights Reserved.Helvetica is a trademark of Linotype-Hell AG and/or its subsidiaries.
//! Helvetica-Bold.afm: Copyright (c) 1985, 1987, 1989, 1990, 1997 Adobe Systems Incorporated.  All Rights Reserved.Helvetica is a trademark of Linotype-Hell AG and/or its subsidiaries.
//! Times-Roman.afm: Copyright (c) 1985, 1987, 1989, 1990, 1993, 1997 Adobe Systems Incorporated.  All Rights Reserved.Times is a trademark of Linotype-Hell AG and/or its subsidiaries.
//! Times-Bold.afm: Copyright (c) 1985, 1987, 1989, 1990, 1993, 1997 Adobe Systems Incorporated.  All Rights Reserved.Times is a trademark of Linotype-Hell AG and/or its subsidiaries.
//! Times-Italic.afm: Copyright (c) 1985, 1987, 1989, 1990, 1993, 1997 Adobe Systems Incorporated.  All Rights Reserved.Times is a trademark of Linotype-Hell AG and/or its subsidiaries.
//! Times-BoldItalic.afm: Copyright (c) 1985, 1987, 1989, 1990, 1993, 1997 Adobe Systems Incorporated.  All Rights Reserved.Times is a trademark of Linotype-Hell AG and/or its subsidiaries.
//! Symbol.afm: Copyright (c) 1985, 1987, 1989, 1990, 1997 Adobe Systems Incorporated. All rights reserved.
//! ZapfDingbats.afm: Copyright (c) 1985, 1987, 1988, 1989, 1997 Adobe Systems Incorporated. All Rights Reserved.ITC Zapf Dingbats is a registered trademark of International Typeface Corporation.
//!
//! The permission notice that accompanies the AFM files, unmodified:
//!
//! This file and the 14 PostScript(R) AFM files it accompanies may be used, copied,
//! and distributed for any purpose and without charge, with or without modification,
//! provided that all copyright notices are retained; that the AFM files are not
//! distributed without this file; that all modifications to this file or any of
//! the AFM files are prominently noted in the modified file(s); and that this
//! paragraph is not modified. Adobe Systems has no responsibility or obligation
//! to support the use of the AFM files.
//!
//! ZapfDingbats' characters are from the ITC Zapf Dingbats Glyph List, whose notice follows,
//! unmodified:
//!
//! Copyright 2002-2019 Adobe (http://www.adobe.com/).
//!
//! Redistribution and use in source and binary forms, with or
//! without modification, are permitted provided that the
//! following conditions are met:
//!
//! Redistributions of source code must retain the above
//! copyright notice, this list of conditions and the following
//! disclaimer.
//!
//! Redistributions in binary form must reproduce the above
//! copyright notice, this list of conditions and the following
//! disclaimer in the documentation and/or other materials
//! provided with the distribution.
//!
//! Neither the name of Adobe nor the names of its contributors
//! may be used to endorse or promote products derived from this
//! software without specific prior written permission.
//!
//! THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND
//! CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//! INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF
//! MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//! DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR
//! CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//! SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT
//! NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES;
//! LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION)
//! HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN
//! CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR
//! OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
//! SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

/// Width (1/1000 em) of `c` in the standard font named `base`, or `None` when the table
/// has no width for it. Courier is 600; a font that is neither Times nor Courier is
/// measured as Helvetica.
pub(crate) fn width(base: &str, c: char) -> Option<f32> {
    let b = base.to_ascii_lowercase();
    if b.contains("courier") {
        return Some(600.0);
    }
    let bold = b.contains("bold");
    let italic = b.contains("italic") || b.contains("oblique");
    let table = if b.contains("times") {
        match (bold, italic) {
            (false, false) => &TIMES_ROMAN,
            (true, false) => &TIMES_BOLD,
            (false, true) => &TIMES_ITALIC,
            (true, true) => &TIMES_BOLD_ITALIC,
        }
    } else if bold {
        &HELVETICA_BOLD
    } else {
        &HELVETICA
    };
    let w = table[code(c)? - 32];
    (w > 0).then_some(w as f32)
}

/// Width (1/1000 em) of `code` in Symbol or ZapfDingbats, which have their own built-in
/// encodings; `None` for other fonts and for codes the font has no glyph for.
pub(crate) fn symbolic_width(base: &str, code: u32) -> Option<f32> {
    let b = base.to_ascii_lowercase();
    let table = if b.contains("dingbats") {
        &ZAPF_DINGBATS
    } else if b.contains("symbol") {
        &SYMBOL
    } else {
        return None;
    };
    let w = *table.get((code as usize).checked_sub(32)?)?;
    (w > 0).then_some(w as f32)
}

/// The character of `code` in the built-in encoding of Symbol or ZapfDingbats (PB-25);
/// `None` for other fonts and for codes the font has no glyph for.
pub(crate) fn symbolic_char(base: &str, code: u8) -> Option<char> {
    let b = base.to_ascii_lowercase();
    let table = if b.contains("dingbats") {
        &ZAPF_DINGBATS_TEXT
    } else if b.contains("symbol") {
        &SYMBOL_TEXT
    } else {
        return None;
    };
    let c = *table.get((code as usize).checked_sub(32)?)?;
    (c != '\0').then_some(c)
}

/// The Windows-1252 code of `c`, from 32 up.
pub(crate) fn code(c: char) -> Option<usize> {
    match c as u32 {
        u @ (0x20..=0x7E | 0xA0..=0xFF) => Some(u as usize),
        _ => CP1252_HIGH
            .iter()
            .position(|&h| h == c && h != '\0')
            .map(|i| 128 + i),
    }
}

// ---------------------------------------------------------------- generated tables

/// Windows-1252 codes 128..=159, as characters; NUL where the code has none.
#[rustfmt::skip]
const CP1252_HIGH: [char; 32] = [
    '\u{20AC}', '\0', '\u{201A}', '\u{0192}', '\u{201E}', '\u{2026}', '\u{2020}', '\u{2021}',
    '\u{02C6}', '\u{2030}', '\u{0160}', '\u{2039}', '\u{0152}', '\0', '\u{017D}', '\0',
    '\0', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}', '\u{2022}', '\u{2013}', '\u{2014}',
    '\u{02DC}', '\u{2122}', '\u{0161}', '\u{203A}', '\u{0153}', '\0', '\u{017E}', '\u{0178}',
];

/// Helvetica: widths (1/1000 em) of Windows-1252 codes 32..=255.
#[rustfmt::skip]
const HELVETICA: [u16; 224] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, // 32-47
    556, 556, 556, 556, 556, 556, 556, 556, 556, 556, 278, 278, 584, 584, 584, 556, // 48-63
    1015, 667, 667, 722, 722, 667, 611, 778, 722, 278, 500, 667, 556, 833, 722, 778, // 64-79
    667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, 278, 278, 278, 469, 556, // 80-95
    333, 556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500, 222, 833, 556, 556, // 96-111
    556, 556, 333, 500, 278, 556, 500, 722, 500, 500, 500, 334, 260, 334, 584, 0, // 112-127
    556, 0, 222, 556, 333, 1000, 556, 556, 333, 1000, 667, 333, 1000, 0, 611, 0, // 128-143
    0, 222, 222, 333, 333, 350, 556, 1000, 333, 1000, 500, 333, 944, 0, 500, 667, // 144-159
    278, 333, 556, 556, 556, 556, 260, 556, 333, 737, 370, 556, 584, 333, 737, 333, // 160-175
    400, 584, 333, 333, 333, 556, 537, 278, 333, 333, 365, 556, 834, 834, 834, 611, // 176-191
    667, 667, 667, 667, 667, 667, 1000, 722, 667, 667, 667, 667, 278, 278, 278, 278, // 192-207
    722, 722, 778, 778, 778, 778, 778, 584, 778, 722, 722, 722, 722, 667, 667, 611, // 208-223
    556, 556, 556, 556, 556, 556, 889, 500, 556, 556, 556, 556, 278, 278, 278, 278, // 224-239
    556, 556, 556, 556, 556, 556, 556, 584, 611, 556, 556, 556, 556, 500, 556, 500, // 240-255
];

/// Helvetica-Bold: widths (1/1000 em) of Windows-1252 codes 32..=255.
#[rustfmt::skip]
const HELVETICA_BOLD: [u16; 224] = [
    278, 333, 474, 556, 556, 889, 722, 238, 333, 333, 389, 584, 278, 333, 278, 278, // 32-47
    556, 556, 556, 556, 556, 556, 556, 556, 556, 556, 333, 333, 584, 584, 584, 611, // 48-63
    975, 722, 722, 722, 722, 667, 611, 778, 722, 278, 556, 722, 611, 833, 722, 778, // 64-79
    667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, 333, 278, 333, 584, 556, // 80-95
    333, 556, 611, 556, 611, 556, 333, 611, 611, 278, 278, 556, 278, 889, 611, 611, // 96-111
    611, 611, 389, 556, 333, 611, 556, 778, 556, 556, 500, 389, 280, 389, 584, 0, // 112-127
    556, 0, 278, 556, 500, 1000, 556, 556, 333, 1000, 667, 333, 1000, 0, 611, 0, // 128-143
    0, 278, 278, 500, 500, 350, 556, 1000, 333, 1000, 556, 333, 944, 0, 500, 667, // 144-159
    278, 333, 556, 556, 556, 556, 280, 556, 333, 737, 370, 556, 584, 333, 737, 333, // 160-175
    400, 584, 333, 333, 333, 611, 556, 278, 333, 333, 365, 556, 834, 834, 834, 611, // 176-191
    722, 722, 722, 722, 722, 722, 1000, 722, 667, 667, 667, 667, 278, 278, 278, 278, // 192-207
    722, 722, 778, 778, 778, 778, 778, 584, 778, 722, 722, 722, 722, 667, 667, 611, // 208-223
    556, 556, 556, 556, 556, 556, 889, 556, 556, 556, 556, 556, 278, 278, 278, 278, // 224-239
    611, 611, 611, 611, 611, 611, 611, 584, 611, 611, 611, 611, 611, 556, 611, 556, // 240-255
];

/// Times-Roman: widths (1/1000 em) of Windows-1252 codes 32..=255.
#[rustfmt::skip]
const TIMES_ROMAN: [u16; 224] = [
    250, 333, 408, 500, 500, 833, 778, 180, 333, 333, 500, 564, 250, 333, 250, 278, // 32-47
    500, 500, 500, 500, 500, 500, 500, 500, 500, 500, 278, 278, 564, 564, 564, 444, // 48-63
    921, 722, 667, 667, 722, 611, 556, 722, 722, 333, 389, 722, 611, 889, 722, 722, // 64-79
    556, 722, 667, 556, 611, 722, 722, 944, 722, 722, 611, 333, 278, 333, 469, 500, // 80-95
    333, 444, 500, 444, 500, 444, 333, 500, 500, 278, 278, 500, 278, 778, 500, 500, // 96-111
    500, 500, 333, 389, 278, 500, 500, 722, 500, 500, 444, 480, 200, 480, 541, 0, // 112-127
    500, 0, 333, 500, 444, 1000, 500, 500, 333, 1000, 556, 333, 889, 0, 611, 0, // 128-143
    0, 333, 333, 444, 444, 350, 500, 1000, 333, 980, 389, 333, 722, 0, 444, 722, // 144-159
    250, 333, 500, 500, 500, 500, 200, 500, 333, 760, 276, 500, 564, 333, 760, 333, // 160-175
    400, 564, 300, 300, 333, 500, 453, 250, 333, 300, 310, 500, 750, 750, 750, 444, // 176-191
    722, 722, 722, 722, 722, 722, 889, 667, 611, 611, 611, 611, 333, 333, 333, 333, // 192-207
    722, 722, 722, 722, 722, 722, 722, 564, 722, 722, 722, 722, 722, 722, 556, 500, // 208-223
    444, 444, 444, 444, 444, 444, 667, 444, 444, 444, 444, 444, 278, 278, 278, 278, // 224-239
    500, 500, 500, 500, 500, 500, 500, 564, 500, 500, 500, 500, 500, 500, 500, 500, // 240-255
];

/// Times-Bold: widths (1/1000 em) of Windows-1252 codes 32..=255.
#[rustfmt::skip]
const TIMES_BOLD: [u16; 224] = [
    250, 333, 555, 500, 500, 1000, 833, 278, 333, 333, 500, 570, 250, 333, 250, 278, // 32-47
    500, 500, 500, 500, 500, 500, 500, 500, 500, 500, 333, 333, 570, 570, 570, 500, // 48-63
    930, 722, 667, 722, 722, 667, 611, 778, 778, 389, 500, 778, 667, 944, 722, 778, // 64-79
    611, 778, 722, 556, 667, 722, 722, 1000, 722, 722, 667, 333, 278, 333, 581, 500, // 80-95
    333, 500, 556, 444, 556, 444, 333, 500, 556, 278, 333, 556, 278, 833, 556, 500, // 96-111
    556, 556, 444, 389, 333, 556, 500, 722, 500, 500, 444, 394, 220, 394, 520, 0, // 112-127
    500, 0, 333, 500, 500, 1000, 500, 500, 333, 1000, 556, 333, 1000, 0, 667, 0, // 128-143
    0, 333, 333, 500, 500, 350, 500, 1000, 333, 1000, 389, 333, 722, 0, 444, 722, // 144-159
    250, 333, 500, 500, 500, 500, 220, 500, 333, 747, 300, 500, 570, 333, 747, 333, // 160-175
    400, 570, 300, 300, 333, 556, 540, 250, 333, 300, 330, 500, 750, 750, 750, 500, // 176-191
    722, 722, 722, 722, 722, 722, 1000, 722, 667, 667, 667, 667, 389, 389, 389, 389, // 192-207
    722, 722, 778, 778, 778, 778, 778, 570, 778, 722, 722, 722, 722, 722, 611, 556, // 208-223
    500, 500, 500, 500, 500, 500, 722, 444, 444, 444, 444, 444, 278, 278, 278, 278, // 224-239
    500, 556, 500, 500, 500, 500, 500, 570, 500, 556, 556, 556, 556, 500, 556, 500, // 240-255
];

/// Times-Italic: widths (1/1000 em) of Windows-1252 codes 32..=255.
#[rustfmt::skip]
const TIMES_ITALIC: [u16; 224] = [
    250, 333, 420, 500, 500, 833, 778, 214, 333, 333, 500, 675, 250, 333, 250, 278, // 32-47
    500, 500, 500, 500, 500, 500, 500, 500, 500, 500, 333, 333, 675, 675, 675, 500, // 48-63
    920, 611, 611, 667, 722, 611, 611, 722, 722, 333, 444, 667, 556, 833, 667, 722, // 64-79
    611, 722, 611, 500, 556, 722, 611, 833, 611, 556, 556, 389, 278, 389, 422, 500, // 80-95
    333, 500, 500, 444, 500, 444, 278, 500, 500, 278, 278, 444, 278, 722, 500, 500, // 96-111
    500, 500, 389, 389, 278, 500, 444, 667, 444, 444, 389, 400, 275, 400, 541, 0, // 112-127
    500, 0, 333, 500, 556, 889, 500, 500, 333, 1000, 500, 333, 944, 0, 556, 0, // 128-143
    0, 333, 333, 556, 556, 350, 500, 889, 333, 980, 389, 333, 667, 0, 389, 556, // 144-159
    250, 389, 500, 500, 500, 500, 275, 500, 333, 760, 276, 500, 675, 333, 760, 333, // 160-175
    400, 675, 300, 300, 333, 500, 523, 250, 333, 300, 310, 500, 750, 750, 750, 500, // 176-191
    611, 611, 611, 611, 611, 611, 889, 667, 611, 611, 611, 611, 333, 333, 333, 333, // 192-207
    722, 667, 722, 722, 722, 722, 722, 675, 722, 722, 722, 722, 722, 556, 611, 500, // 208-223
    500, 500, 500, 500, 500, 500, 667, 444, 444, 444, 444, 444, 278, 278, 278, 278, // 224-239
    500, 500, 500, 500, 500, 500, 500, 675, 500, 500, 500, 500, 500, 444, 500, 444, // 240-255
];

/// Times-BoldItalic: widths (1/1000 em) of Windows-1252 codes 32..=255.
#[rustfmt::skip]
const TIMES_BOLD_ITALIC: [u16; 224] = [
    250, 389, 555, 500, 500, 833, 778, 278, 333, 333, 500, 570, 250, 333, 250, 278, // 32-47
    500, 500, 500, 500, 500, 500, 500, 500, 500, 500, 333, 333, 570, 570, 570, 500, // 48-63
    832, 667, 667, 667, 722, 667, 667, 722, 778, 389, 500, 667, 611, 889, 722, 722, // 64-79
    611, 722, 667, 556, 611, 722, 667, 889, 667, 611, 611, 333, 278, 333, 570, 500, // 80-95
    333, 500, 500, 444, 500, 444, 333, 500, 556, 278, 278, 500, 278, 778, 556, 500, // 96-111
    500, 500, 389, 389, 278, 556, 444, 667, 500, 444, 389, 348, 220, 348, 570, 0, // 112-127
    500, 0, 333, 500, 500, 1000, 500, 500, 333, 1000, 556, 333, 944, 0, 611, 0, // 128-143
    0, 333, 333, 500, 500, 350, 500, 1000, 333, 1000, 389, 333, 722, 0, 389, 611, // 144-159
    250, 389, 500, 500, 500, 500, 220, 500, 333, 747, 266, 500, 606, 333, 747, 333, // 160-175
    400, 570, 300, 300, 333, 576, 500, 250, 333, 300, 300, 500, 750, 750, 750, 500, // 176-191
    667, 667, 667, 667, 667, 667, 944, 667, 667, 667, 667, 667, 389, 389, 389, 389, // 192-207
    722, 722, 722, 722, 722, 722, 722, 570, 722, 722, 722, 722, 722, 611, 611, 500, // 208-223
    500, 500, 500, 500, 500, 500, 722, 444, 444, 444, 444, 444, 278, 278, 278, 278, // 224-239
    500, 556, 500, 500, 500, 500, 500, 570, 500, 556, 556, 556, 556, 444, 500, 444, // 240-255
];

/// Symbol: widths (1/1000 em) of the codes of its built-in encoding 32..=255.
#[rustfmt::skip]
const SYMBOL: [u16; 224] = [
    250, 333, 713, 500, 549, 833, 778, 439, 333, 333, 500, 549, 250, 549, 250, 278, // 32-47
    500, 500, 500, 500, 500, 500, 500, 500, 500, 500, 278, 278, 549, 549, 549, 444, // 48-63
    549, 722, 667, 722, 612, 611, 763, 603, 722, 333, 631, 722, 686, 889, 722, 722, // 64-79
    768, 741, 556, 592, 611, 690, 439, 768, 645, 795, 611, 333, 863, 333, 658, 500, // 80-95
    500, 631, 549, 549, 494, 439, 521, 411, 603, 329, 603, 549, 549, 576, 521, 549, // 96-111
    549, 521, 549, 603, 439, 576, 713, 686, 493, 686, 494, 480, 200, 480, 549, 0, // 112-127
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // 128-143
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // 144-159
    750, 620, 247, 549, 167, 713, 500, 753, 753, 753, 753, 1042, 987, 603, 987, 603, // 160-175
    400, 549, 411, 549, 549, 713, 494, 460, 549, 549, 549, 549, 1000, 603, 1000, 658, // 176-191
    823, 686, 795, 987, 768, 768, 823, 768, 768, 713, 713, 713, 713, 713, 713, 713, // 192-207
    768, 713, 790, 790, 890, 823, 549, 250, 713, 603, 603, 1042, 987, 603, 987, 603, // 208-223
    494, 329, 790, 790, 786, 713, 384, 384, 384, 384, 384, 384, 494, 494, 494, 494, // 224-239
    0, 329, 274, 686, 686, 686, 384, 384, 384, 384, 384, 384, 494, 494, 494, 0, // 240-255
];

/// ZapfDingbats: widths (1/1000 em) of the codes of its built-in encoding 32..=255.
#[rustfmt::skip]
const ZAPF_DINGBATS: [u16; 224] = [
    278, 974, 961, 974, 980, 719, 789, 790, 791, 690, 960, 939, 549, 855, 911, 933, // 32-47
    911, 945, 974, 755, 846, 762, 761, 571, 677, 763, 760, 759, 754, 494, 552, 537, // 48-63
    577, 692, 786, 788, 788, 790, 793, 794, 816, 823, 789, 841, 823, 833, 816, 831, // 64-79
    923, 744, 723, 749, 790, 792, 695, 776, 768, 792, 759, 707, 708, 682, 701, 826, // 80-95
    815, 789, 789, 707, 687, 696, 689, 786, 787, 713, 791, 785, 791, 873, 761, 762, // 96-111
    762, 759, 759, 892, 892, 788, 784, 438, 138, 277, 415, 392, 392, 668, 668, 0, // 112-127
    390, 390, 317, 317, 276, 276, 509, 509, 410, 410, 234, 234, 334, 334, 0, 0, // 128-143
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // 144-159
    0, 732, 544, 544, 910, 667, 760, 760, 776, 595, 694, 626, 788, 788, 788, 788, // 160-175
    788, 788, 788, 788, 788, 788, 788, 788, 788, 788, 788, 788, 788, 788, 788, 788, // 176-191
    788, 788, 788, 788, 788, 788, 788, 788, 788, 788, 788, 788, 788, 788, 788, 788, // 192-207
    788, 788, 788, 788, 894, 838, 1016, 458, 748, 924, 748, 918, 927, 928, 928, 834, // 208-223
    873, 828, 924, 924, 917, 930, 931, 463, 883, 836, 836, 867, 867, 696, 696, 874, // 224-239
    0, 874, 760, 946, 771, 865, 771, 888, 967, 888, 831, 873, 927, 970, 918, 0, // 240-255
];

/// Symbol: the character of each code of its built-in encoding 32..=255, by its
/// glyph's name; NUL where the code has none.
#[rustfmt::skip]
const SYMBOL_TEXT: [char; 224] = [
    '\u{0020}', '\u{0021}', '\u{2200}', '\u{0023}', '\u{2203}', '\u{0025}', '\u{0026}', '\u{220B}', // 32-39
    '\u{0028}', '\u{0029}', '\u{2217}', '\u{002B}', '\u{002C}', '\u{2212}', '\u{002E}', '\u{002F}', // 40-47
    '\u{0030}', '\u{0031}', '\u{0032}', '\u{0033}', '\u{0034}', '\u{0035}', '\u{0036}', '\u{0037}', // 48-55
    '\u{0038}', '\u{0039}', '\u{003A}', '\u{003B}', '\u{003C}', '\u{003D}', '\u{003E}', '\u{003F}', // 56-63
    '\u{2245}', '\u{0391}', '\u{0392}', '\u{03A7}', '\u{2206}', '\u{0395}', '\u{03A6}', '\u{0393}', // 64-71
    '\u{0397}', '\u{0399}', '\u{03D1}', '\u{039A}', '\u{039B}', '\u{039C}', '\u{039D}', '\u{039F}', // 72-79
    '\u{03A0}', '\u{0398}', '\u{03A1}', '\u{03A3}', '\u{03A4}', '\u{03A5}', '\u{03C2}', '\u{2126}', // 80-87
    '\u{039E}', '\u{03A8}', '\u{0396}', '\u{005B}', '\u{2234}', '\u{005D}', '\u{22A5}', '\u{005F}', // 88-95
    '\u{F8E5}', '\u{03B1}', '\u{03B2}', '\u{03C7}', '\u{03B4}', '\u{03B5}', '\u{03C6}', '\u{03B3}', // 96-103
    '\u{03B7}', '\u{03B9}', '\u{03D5}', '\u{03BA}', '\u{03BB}', '\u{00B5}', '\u{03BD}', '\u{03BF}', // 104-111
    '\u{03C0}', '\u{03B8}', '\u{03C1}', '\u{03C3}', '\u{03C4}', '\u{03C5}', '\u{03D6}', '\u{03C9}', // 112-119
    '\u{03BE}', '\u{03C8}', '\u{03B6}', '\u{007B}', '\u{007C}', '\u{007D}', '\u{223C}', '\0', // 120-127
    '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', // 128-135
    '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', // 136-143
    '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', // 144-151
    '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', // 152-159
    '\u{20AC}', '\u{03D2}', '\u{2032}', '\u{2264}', '\u{2044}', '\u{221E}', '\u{0192}', '\u{2663}', // 160-167
    '\u{2666}', '\u{2665}', '\u{2660}', '\u{2194}', '\u{2190}', '\u{2191}', '\u{2192}', '\u{2193}', // 168-175
    '\u{00B0}', '\u{00B1}', '\u{2033}', '\u{2265}', '\u{00D7}', '\u{221D}', '\u{2202}', '\u{2022}', // 176-183
    '\u{00F7}', '\u{2260}', '\u{2261}', '\u{2248}', '\u{2026}', '\u{F8E6}', '\u{F8E7}', '\u{21B5}', // 184-191
    '\u{2135}', '\u{2111}', '\u{211C}', '\u{2118}', '\u{2297}', '\u{2295}', '\u{2205}', '\u{2229}', // 192-199
    '\u{222A}', '\u{2283}', '\u{2287}', '\u{2284}', '\u{2282}', '\u{2286}', '\u{2208}', '\u{2209}', // 200-207
    '\u{2220}', '\u{2207}', '\u{F6DA}', '\u{F6D9}', '\u{F6DB}', '\u{220F}', '\u{221A}', '\u{22C5}', // 208-215
    '\u{00AC}', '\u{2227}', '\u{2228}', '\u{21D4}', '\u{21D0}', '\u{21D1}', '\u{21D2}', '\u{21D3}', // 216-223
    '\u{25CA}', '\u{2329}', '\u{F8E8}', '\u{F8E9}', '\u{F8EA}', '\u{2211}', '\u{F8EB}', '\u{F8EC}', // 224-231
    '\u{F8ED}', '\u{F8EE}', '\u{F8EF}', '\u{F8F0}', '\u{F8F1}', '\u{F8F2}', '\u{F8F3}', '\u{F8F4}', // 232-239
    '\0', '\u{232A}', '\u{222B}', '\u{2320}', '\u{F8F5}', '\u{2321}', '\u{F8F6}', '\u{F8F7}', // 240-247
    '\u{F8F8}', '\u{F8F9}', '\u{F8FA}', '\u{F8FB}', '\u{F8FC}', '\u{F8FD}', '\u{F8FE}', '\0', // 248-255
];

/// ZapfDingbats: the character of each code of its built-in encoding 32..=255, by its
/// glyph's name; NUL where the code has none.
#[rustfmt::skip]
const ZAPF_DINGBATS_TEXT: [char; 224] = [
    '\u{0020}', '\u{2701}', '\u{2702}', '\u{2703}', '\u{2704}', '\u{260E}', '\u{2706}', '\u{2707}', // 32-39
    '\u{2708}', '\u{2709}', '\u{261B}', '\u{261E}', '\u{270C}', '\u{270D}', '\u{270E}', '\u{270F}', // 40-47
    '\u{2710}', '\u{2711}', '\u{2712}', '\u{2713}', '\u{2714}', '\u{2715}', '\u{2716}', '\u{2717}', // 48-55
    '\u{2718}', '\u{2719}', '\u{271A}', '\u{271B}', '\u{271C}', '\u{271D}', '\u{271E}', '\u{271F}', // 56-63
    '\u{2720}', '\u{2721}', '\u{2722}', '\u{2723}', '\u{2724}', '\u{2725}', '\u{2726}', '\u{2727}', // 64-71
    '\u{2605}', '\u{2729}', '\u{272A}', '\u{272B}', '\u{272C}', '\u{272D}', '\u{272E}', '\u{272F}', // 72-79
    '\u{2730}', '\u{2731}', '\u{2732}', '\u{2733}', '\u{2734}', '\u{2735}', '\u{2736}', '\u{2737}', // 80-87
    '\u{2738}', '\u{2739}', '\u{273A}', '\u{273B}', '\u{273C}', '\u{273D}', '\u{273E}', '\u{273F}', // 88-95
    '\u{2740}', '\u{2741}', '\u{2742}', '\u{2743}', '\u{2744}', '\u{2745}', '\u{2746}', '\u{2747}', // 96-103
    '\u{2748}', '\u{2749}', '\u{274A}', '\u{274B}', '\u{25CF}', '\u{274D}', '\u{25A0}', '\u{274F}', // 104-111
    '\u{2750}', '\u{2751}', '\u{2752}', '\u{25B2}', '\u{25BC}', '\u{25C6}', '\u{2756}', '\u{25D7}', // 112-119
    '\u{2758}', '\u{2759}', '\u{275A}', '\u{275B}', '\u{275C}', '\u{275D}', '\u{275E}', '\0', // 120-127
    '\u{2768}', '\u{2769}', '\u{276A}', '\u{276B}', '\u{276C}', '\u{276D}', '\u{276E}', '\u{276F}', // 128-135
    '\u{2770}', '\u{2771}', '\u{2772}', '\u{2773}', '\u{2774}', '\u{2775}', '\0', '\0', // 136-143
    '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', // 144-151
    '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', // 152-159
    '\0', '\u{2761}', '\u{2762}', '\u{2763}', '\u{2764}', '\u{2765}', '\u{2766}', '\u{2767}', // 160-167
    '\u{2663}', '\u{2666}', '\u{2665}', '\u{2660}', '\u{2460}', '\u{2461}', '\u{2462}', '\u{2463}', // 168-175
    '\u{2464}', '\u{2465}', '\u{2466}', '\u{2467}', '\u{2468}', '\u{2469}', '\u{2776}', '\u{2777}', // 176-183
    '\u{2778}', '\u{2779}', '\u{277A}', '\u{277B}', '\u{277C}', '\u{277D}', '\u{277E}', '\u{277F}', // 184-191
    '\u{2780}', '\u{2781}', '\u{2782}', '\u{2783}', '\u{2784}', '\u{2785}', '\u{2786}', '\u{2787}', // 192-199
    '\u{2788}', '\u{2789}', '\u{278A}', '\u{278B}', '\u{278C}', '\u{278D}', '\u{278E}', '\u{278F}', // 200-207
    '\u{2790}', '\u{2791}', '\u{2792}', '\u{2793}', '\u{2794}', '\u{2192}', '\u{2194}', '\u{2195}', // 208-215
    '\u{2798}', '\u{2799}', '\u{279A}', '\u{279B}', '\u{279C}', '\u{279D}', '\u{279E}', '\u{279F}', // 216-223
    '\u{27A0}', '\u{27A1}', '\u{27A2}', '\u{27A3}', '\u{27A4}', '\u{27A5}', '\u{27A6}', '\u{27A7}', // 224-231
    '\u{27A8}', '\u{27A9}', '\u{27AA}', '\u{27AB}', '\u{27AC}', '\u{27AD}', '\u{27AE}', '\u{27AF}', // 232-239
    '\0', '\u{27B1}', '\u{27B2}', '\u{27B3}', '\u{27B4}', '\u{27B5}', '\u{27B6}', '\u{27B7}', // 240-247
    '\u{27B8}', '\u{27B9}', '\u{27BA}', '\u{27BB}', '\u{27BC}', '\u{27BD}', '\u{27BE}', '\0', // 248-255
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widths_by_font_and_character() {
        assert_eq!(width("Helvetica", 'W'), Some(944.0));
        assert_eq!(width("Helvetica-BoldOblique", 'a'), Some(556.0));
        assert_eq!(width("Times-Roman", 'i'), Some(278.0));
        assert_eq!(width("Times-Italic", 'm'), Some(722.0));
        assert_eq!(width("Times-BoldItalic", 'A'), Some(667.0));
        assert_eq!(width("Courier-Bold", '\u{4E2D}'), Some(600.0));
        // Beyond ASCII: Latin-1 and the Windows-1252 punctuation.
        assert_eq!(width("Times-Roman", 'é'), Some(444.0));
        assert_eq!(width("Helvetica", '\u{2014}'), Some(1000.0));
        assert_eq!(width("Helvetica", '\u{20AC}'), Some(556.0));
        assert_eq!(width("Helvetica", '\u{4E2D}'), None);
        assert_eq!(width("Helvetica", '\u{7F}'), None);
    }

    #[test]
    fn symbolic_widths_by_code() {
        // Symbol 0x61 is alpha (631); ZapfDingbats 0x21 is a pair of scissors (974).
        assert_eq!(symbolic_width("Symbol", 0x61), Some(631.0));
        assert_eq!(symbolic_width("ZapfDingbats", 0x21), Some(974.0));
        assert_eq!(symbolic_width("Symbol", 0x80), None);
        assert_eq!(symbolic_width("Symbol", 10), None);
        assert_eq!(symbolic_width("Times-Roman", 0x61), None);
    }
}

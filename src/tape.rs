use anyhow::{Result, bail};

/// Print resolution of SR5500P (dots per inch, both directions).
pub const DPI: u32 = 180;

pub fn mm_to_dots(mm: f32) -> usize {
    (mm * DPI as f32 / 25.4).round() as usize
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Tape {
    W4,
    W6,
    W9,
    W12,
    W18,
    W24,
}

impl Tape {
    pub fn from_mm(mm: u32) -> Result<Self> {
        Ok(match mm {
            4 => Tape::W4,
            6 => Tape::W6,
            9 => Tape::W9,
            12 => Tape::W12,
            18 => Tape::W18,
            24 => Tape::W24,
            _ => bail!("unsupported tape width: {mm} mm"),
        })
    }

    /// Decode the tape width code reported in the printer status.
    /// Returns `None` for "no tape" and unknown codes.
    pub fn from_status_code(code: u8) -> Option<Self> {
        Some(match code {
            0x01 => Tape::W6,
            0x02 => Tape::W9,
            0x03 => Tape::W12,
            0x04 => Tape::W18,
            0x05 => Tape::W24,
            0x0b => Tape::W4,
            _ => return None,
        })
    }

    pub fn mm(&self) -> u32 {
        match self {
            Tape::W4 => 4,
            Tape::W6 => 6,
            Tape::W9 => 9,
            Tape::W12 => 12,
            Tape::W18 => 18,
            Tape::W24 => 24,
        }
    }

    /// Number of printable dots across the tape at 180 dpi
    /// (`PrintableSizeTable` of the TEPRA SDK; 9 mm verified on SR5500P).
    pub fn printable_dots(&self) -> usize {
        match self {
            Tape::W4 => 22,
            Tape::W6 => 36,
            Tape::W9 => 54,
            Tape::W12 => 72,
            Tape::W18 => 108,
            Tape::W24 => 128,
        }
    }
}

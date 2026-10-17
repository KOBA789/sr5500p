//! Print job encoding for TEPRA PRO SR5500P (commandLevel 1 in the TEPRA SDK).
//!
//! Control commands are framed as `1B 7B <len> <cmd> <params...> <sum> 7D`,
//! where `len` counts the bytes from `cmd` to `7D` and `sum` is the low byte of
//! `cmd + Σparams`. Raster data follows as one `ESC .` line per column.

use crate::bitmap::Bitmap;
use anyhow::{Result, bail};
use std::fmt::Write;

#[derive(Copy, Clone, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Cut {
    /// cut after each label
    Each,
    /// cut once after the job
    Job,
    /// do not cut
    None,
}

impl Cut {
    // The TEPRA SDK sends 01 01 01 01 for "cut each label", but SR5500P then
    // cuts about 10 mm before the end of the label. 02 02 01 01 (what SPC10
    // sends to SR920/SR5900P) works. Job is taken from SR920 captures and
    // is not verified on SR5500P.
    fn params(&self) -> [u8; 4] {
        match self {
            Cut::Each => [0x02, 0x02, 0x01, 0x01],
            Cut::Job => [0x02, 0x00, 0x01, 0x01],
            Cut::None => [0x00, 0x00, 0x00, 0x00],
        }
    }
}

#[derive(Copy, Clone, Debug)]
pub struct JobOptions {
    pub cut: Cut,
    /// -5 (lightest) ..= 5 (darkest), 0 is the default
    pub density: i8,
    /// blank feed before the image, in dots
    pub margin: u16,
}

fn command(out: &mut Vec<u8>, cmd: u8, params: &[u8]) {
    let sum = params.iter().fold(cmd, |a, b| a.wrapping_add(*b));
    out.extend_from_slice(&[0x1b, 0x7b, (params.len() + 3) as u8, cmd]);
    out.extend_from_slice(params);
    out.extend_from_slice(&[sum, 0x7d]);
}

/// Encode `labels` as a single job. Each bitmap's height must equal the
/// printable dots of the loaded tape.
pub fn encode_job(labels: &[Bitmap], opts: &JobOptions) -> Vec<u8> {
    let mut out = Vec::new();
    // job environment
    command(&mut out, b'@', &[]);
    command(&mut out, b'{', &[0x00, 0x00, b'S', b'T']);
    command(&mut out, b'C', &opts.cut.params());
    command(&mut out, b'D', &[(opts.density.clamp(-5, 5) + 5) as u8]);
    command(&mut out, b'G', &[]);
    for bmp in labels {
        // page environment
        command(&mut out, b'L', &(bmp.width as u32).to_le_bytes());
        command(&mut out, b'T', &opts.margin.to_le_bytes());
        // Columns are sent from the trailing end of the label. Within a
        // column the MSB of the first byte is the top edge of the tape.
        let bytes_per_col = bmp.height.div_ceil(8);
        for x in (0..bmp.width).rev() {
            out.extend_from_slice(&[0x1b, 0x2e, 0x00, 0x00, 0x00, 0x01]);
            out.extend_from_slice(&(bmp.height as u16).to_le_bytes());
            for b in 0..bytes_per_col {
                let mut v = 0u8;
                for bit in 0..8 {
                    if bmp.get(x, b * 8 + bit) {
                        v |= 0x80 >> bit;
                    }
                }
                out.push(v);
            }
        }
        out.push(0x0c);
    }
    command(&mut out, b'@', &[]);
    out
}

/// Human-readable dump of an encoded job (for --dry-run).
pub fn describe(data: &[u8]) -> Result<String> {
    let mut s = String::new();
    let mut i = 0;
    let mut cols = 0usize;
    let flush_cols = |s: &mut String, cols: &mut usize| {
        if *cols > 0 {
            writeln!(s, "  raster x {cols}").unwrap();
            *cols = 0;
        }
    };
    while i < data.len() {
        match data[i..] {
            [0x1b, 0x7b, len, ..] => {
                let end = i + 3 + len as usize;
                if end > data.len() || data[end - 1] != 0x7d {
                    bail!("broken command frame at {i:#x}");
                }
                let body = &data[i + 3..end - 2];
                let sum = body.iter().fold(0u8, |a, b| a.wrapping_add(*b));
                if sum != data[end - 2] {
                    bail!("checksum mismatch at {i:#x}");
                }
                flush_cols(&mut s, &mut cols);
                writeln!(s, "  cmd '{}' {:02x?}", body[0] as char, &body[1..]).unwrap();
                i = end;
            }
            [0x1b, 0x2e, 0, 0, 0, 1, lo, hi, ..] => {
                let n = u16::from_le_bytes([lo, hi]) as usize;
                i += 8 + n.div_ceil(8);
                if i > data.len() {
                    bail!("truncated raster line");
                }
                cols += 1;
            }
            [0x0c, ..] => {
                flush_cols(&mut s, &mut cols);
                writeln!(s, "  end of page").unwrap();
                i += 1;
            }
            _ => bail!("unknown byte {:#04x} at {i:#x}", data[i]),
        }
    }
    if i != data.len() {
        bail!("truncated data");
    }
    Ok(s)
}

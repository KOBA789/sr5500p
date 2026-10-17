mod bitmap;
mod device;
mod protocol;
mod tape;

use anyhow::{Context, Result, bail};
use bitmap::Bitmap;
use clap::{Args, Parser, Subcommand};
use device::Printer;
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::mono_font::ascii::FONT_10X20;
use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{PrimitiveStyle, Rectangle};
use embedded_graphics::text::{Baseline, Text};
use protocol::{Cut, JobOptions};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};
use tape::{Tape, mm_to_dots};

/// Label printing tool for KING JIM TEPRA PRO SR5500P (USB)
#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Show the printer status
    Status,
    /// Print a label
    Print {
        #[command(flatten)]
        opts: PrintOpts,
        #[command(subcommand)]
        content: Content,
    },
}

#[derive(Args)]
struct PrintOpts {
    /// tape width in mm (default: detect from the printer)
    #[arg(long, global = true)]
    tape: Option<u32>,
    /// generate the preview and dump the data without printing
    #[arg(long, global = true)]
    dry_run: bool,
    /// where to write the preview image
    #[arg(long, global = true, default_value = "preview.png")]
    preview: PathBuf,
    /// cut mode
    #[arg(long, global = true, value_enum, default_value_t = Cut::Each)]
    cut: Cut,
    /// print density, -5 (light) to 5 (dark)
    #[arg(long, global = true, default_value_t = 0, allow_negative_numbers = true,
          value_parser = clap::value_parser!(i8).range(-5..=5))]
    density: i8,
    /// blank space before the image, in mm
    #[arg(long, global = true, default_value_t = 0.0)]
    margin: f32,
    /// also write the encoded job to this file
    #[arg(long, global = true)]
    output: Option<PathBuf>,
    /// number of copies
    #[arg(long, global = true, default_value_t = 1)]
    copies: usize,
}

#[derive(Subcommand)]
enum Content {
    /// Outline, millimeter ticks and a row ruler
    TestPattern {
        /// label length in mm
        #[arg(long, default_value_t = 30.0)]
        length: f32,
    },
    /// ASCII text, scaled to fill the tape (one label per argument)
    Text {
        #[arg(required = true)]
        texts: Vec<String>,
    },
    /// QR code followed by the text (one label per argument)
    Qr {
        #[arg(required = true)]
        texts: Vec<String>,
    },
    /// Image files, scaled to the tape width (one label per file)
    Image {
        #[arg(required = true)]
        paths: Vec<PathBuf>,
        /// gray level below which a pixel is printed (0-255)
        #[arg(long, default_value_t = 140)]
        threshold: u8,
        /// rotate the image 90 degrees counter-clockwise first
        #[arg(long)]
        rotate: bool,
    },
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Status => status(),
        Command::Print { opts, content } => print(&opts, &content),
    }
}

fn status() -> Result<()> {
    let printer = Printer::find()?;
    let st = printer.status()?;
    println!("device: {} ({}), serial {}", printer.lp.display(), printer.usbfs.display(), printer.serial);
    println!("state:  {} ({})", st.state(), st.state_name());
    println!("error:  {:#04x} ({})", st.error(), st.error_name());
    match st.tape() {
        Some(t) => println!("tape:   {} mm ({} printable dots)", t.mm(), t.printable_dots()),
        None => println!("tape:   none / unknown (code {:#04x})", st.raw[3]),
    }
    println!("raw:    {:02x?}", st.raw);
    Ok(())
}

fn print(opts: &PrintOpts, content: &Content) -> Result<()> {
    let printer = if opts.dry_run && opts.tape.is_some() { None } else { Some(Printer::find()?) };
    let tape = determine_tape(opts, printer.as_ref())?;
    let labels = render(content, tape.printable_dots())?;
    preview(&labels).save_png(&opts.preview)?;
    eprintln!("{} mm tape, preview: {}", tape.mm(), opts.preview.display());
    for label in &labels {
        eprintln!(
            "  label {:.1} mm ({} x {} dots)",
            label.width as f32 * 25.4 / tape::DPI as f32,
            label.width,
            label.height
        );
    }
    // All labels go into one job: the printer wastes ~12 mm of tape at the
    // start of every job, but none between labels within a job.
    let labels: Vec<Bitmap> = (0..opts.copies).flat_map(|_| labels.iter().cloned()).collect();
    let job = protocol::encode_job(
        &labels,
        &JobOptions {
            cut: opts.cut,
            density: opts.density,
            margin: mm_to_dots(opts.margin) as u16,
        },
    );
    if let Some(path) = &opts.output {
        std::fs::write(path, &job)?;
    }
    if opts.dry_run {
        print!("{}", protocol::describe(&job)?);
        return Ok(());
    }
    let printer = printer.unwrap();
    printer.send(&job)?;
    wait_for_completion(&printer)
}

fn determine_tape(opts: &PrintOpts, printer: Option<&Printer>) -> Result<Tape> {
    let given = opts.tape.map(Tape::from_mm).transpose()?;
    let Some(printer) = printer else {
        return Ok(given.unwrap());
    };
    let st = printer.status()?;
    if st.error() != 0 {
        bail!("printer error: {:#04x} ({})", st.error(), st.error_name());
    }
    match (given, st.tape()) {
        (Some(g), Some(d)) if g != d && !opts.dry_run => {
            bail!("--tape {} mm does not match the loaded {} mm tape", g.mm(), d.mm())
        }
        (Some(g), _) => Ok(g),
        (None, Some(d)) => Ok(d),
        (None, None) => bail!("no tape detected (status {:02x?})", st.raw),
    }
}

fn wait_for_completion(printer: &Printer) -> Result<()> {
    let start = Instant::now();
    loop {
        let st = printer.status()?;
        if st.error() != 0 {
            bail!("printer error: {:#04x} ({})", st.error(), st.error_name());
        }
        if !matches!(st.state(), 1..=3) {
            eprintln!("done ({})", st.state_name());
            return Ok(());
        }
        if start.elapsed() > Duration::from_secs(120) {
            bail!("timed out waiting for the printer (status {:02x?})", st.raw);
        }
        thread::sleep(Duration::from_millis(300));
    }
}

/// Stack the labels vertically with a small gap.
fn preview(labels: &[Bitmap]) -> Bitmap {
    const GAP: usize = 4;
    let width = labels.iter().map(|l| l.width).max().unwrap_or(0);
    let height = labels.iter().map(|l| l.height + GAP).sum::<usize>() - GAP;
    let mut bmp = Bitmap::new(width, height);
    let mut y = 0;
    for l in labels {
        bmp.blit(l, 0, y);
        y += l.height + GAP;
    }
    bmp
}

fn render(content: &Content, height: usize) -> Result<Vec<Bitmap>> {
    match content {
        Content::TestPattern { length } => Ok(vec![test_pattern(mm_to_dots(*length), height)]),
        Content::Text { texts } => Ok(texts.iter().map(|t| text_label(t, height)).collect()),
        Content::Qr { texts } => texts.iter().map(|t| qr_label(t, height)).collect(),
        Content::Image { paths, threshold, rotate } => {
            paths.iter().map(|p| image_label(p, *threshold, *rotate, height)).collect()
        }
    }
}

fn text_label(text: &str, height: usize) -> Bitmap {
    let t = text_bitmap(text, height);
    let pad = height / 4;
    let mut bmp = Bitmap::new(t.width + pad * 2, height);
    bmp.blit(&t, pad, (height - t.height) / 2);
    bmp
}

fn qr_label(text: &str, height: usize) -> Result<Bitmap> {
    let q = qr_bitmap(text, height)?;
    let t = text_bitmap(text, height);
    let gap = height / 8;
    let mut bmp = Bitmap::new(q.width + gap + t.width + gap, height);
    bmp.blit(&q, 0, (height - q.height) / 2);
    bmp.blit(&t, q.width + gap, (height - t.height) / 2);
    Ok(bmp)
}

fn image_label(path: &Path, threshold: u8, rotate: bool, height: usize) -> Result<Bitmap> {
    let img = image::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let img = if rotate { img.rotate270() } else { img };
    let img = img.resize(u32::MAX, height as u32, image::imageops::FilterType::Triangle).to_luma8();
    let mut bmp = Bitmap::new(img.width() as usize, height);
    let top = (height - img.height() as usize) / 2;
    for (x, y, p) in img.enumerate_pixels() {
        bmp.set(x as usize, top + y as usize, p.0[0] < threshold);
    }
    Ok(bmp)
}

/// Render ASCII text with the 10x20 font, magnified to fit `height`.
fn text_bitmap(text: &str, height: usize) -> Bitmap {
    let style = MonoTextStyle::new(&FONT_10X20, BinaryColor::On);
    let mut bmp = Bitmap::new(10 * text.chars().count(), 20);
    Text::with_baseline(text, Point::zero(), style, Baseline::Top).draw(&mut bmp).unwrap();
    bmp.scaled(height / 20)
}

fn qr_bitmap(text: &str, height: usize) -> Result<Bitmap> {
    let code = qrcode::QrCode::new(text.as_bytes())?;
    let n = code.width();
    let colors = code.to_colors();
    // one module of quiet zone on each side is enough on a white tape
    let r = height / (n + 2);
    if r == 0 {
        bail!("QR code ({n} modules) does not fit in {height} dots");
    }
    let mut bmp = Bitmap::new(height, height);
    let ofs = (height - n * r) / 2;
    for (i, c) in colors.iter().enumerate() {
        if *c == qrcode::Color::Dark {
            let (mx, my) = (i % n, i / n);
            for dy in 0..r {
                for dx in 0..r {
                    bmp.set(ofs + mx * r + dx, ofs + my * r + dy, true);
                }
            }
        }
    }
    Ok(bmp)
}

fn test_pattern(width: usize, height: usize) -> Bitmap {
    let mut bmp = Bitmap::new(width, height);
    let on = PrimitiveStyle::with_stroke(BinaryColor::On, 1);
    Rectangle::new(Point::zero(), Size::new(width as u32, height as u32))
        .into_styled(on)
        .draw(&mut bmp)
        .unwrap();
    // millimeter ticks along the top and bottom edges, longer every 5 mm
    for mm in 0.. {
        let x = mm_to_dots(mm as f32);
        if x >= width {
            break;
        }
        let len = if mm % 10 == 0 { 8 } else if mm % 5 == 0 { 5 } else { 3 };
        for y in 0..len.min(height / 3) {
            bmp.set(x, y, true);
            bmp.set(x, height - 1 - y, true);
        }
    }
    // row ruler: every 8th row has a short bar, longer every 32 rows
    for y in (0..height).step_by(8) {
        let len = if y % 32 == 0 { 12 } else { 6 };
        for x in 12..12 + len {
            bmp.set(x, y, true);
        }
    }
    let t = text_bitmap("Ag", height * 2 / 3);
    bmp.blit(&t, (width.saturating_sub(t.width)) / 2, (height - t.height) / 2);
    bmp
}

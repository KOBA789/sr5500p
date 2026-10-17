use anyhow::Result;
use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::prelude::*;
use std::convert::Infallible;
use std::path::Path;

/// A 1-bit image in label coordinates:
/// x runs along the tape (label length), y runs across the tape (0 = top edge).
#[derive(Clone)]
pub struct Bitmap {
    pub width: usize,
    pub height: usize,
    data: Vec<bool>,
}

impl Bitmap {
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            data: vec![false; width * height],
        }
    }

    pub fn get(&self, x: usize, y: usize) -> bool {
        x < self.width && y < self.height && self.data[y * self.width + x]
    }

    pub fn set(&mut self, x: usize, y: usize, v: bool) {
        if x < self.width && y < self.height {
            self.data[y * self.width + x] = v;
        }
    }

    pub fn scaled(&self, r: usize) -> Self {
        let r = r.max(1);
        let mut new = Self::new(self.width * r, self.height * r);
        for y in 0..new.height {
            for x in 0..new.width {
                new.set(x, y, self.get(x / r, y / r));
            }
        }
        new
    }

    /// OR `src` onto self with its top-left corner at (x, y).
    pub fn blit(&mut self, src: &Bitmap, x: usize, y: usize) {
        for sy in 0..src.height {
            for sx in 0..src.width {
                if src.get(sx, sy) {
                    self.set(x + sx, y + sy, true);
                }
            }
        }
    }

    pub fn save_png(&self, path: &Path) -> Result<()> {
        let img = image::GrayImage::from_fn(self.width as u32, self.height as u32, |x, y| {
            image::Luma([if self.get(x as usize, y as usize) { 0 } else { 255 }])
        });
        img.save(path)?;
        Ok(())
    }
}

impl OriginDimensions for Bitmap {
    fn size(&self) -> Size {
        Size::new(self.width as u32, self.height as u32)
    }
}

impl DrawTarget for Bitmap {
    type Color = BinaryColor;
    type Error = Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        for Pixel(p, c) in pixels {
            if p.x >= 0 && p.y >= 0 {
                self.set(p.x as usize, p.y as usize, c.is_on());
            }
        }
        Ok(())
    }
}

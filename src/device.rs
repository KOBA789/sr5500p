//! Access to the printer through the Linux USB stack.
//!
//! Print data goes through the usblp character device (`/dev/usb/lpN`).
//! Status is read with a vendor control request over usbfs
//! (`/dev/bus/usb/BBB/DDD`), which works while usblp stays bound.

use crate::tape::Tape;
use anyhow::{Context, Result, anyhow, bail};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};

const VENDOR_ID: &str = "0d8a";
const PRODUCT_ID: &str = "0103";

pub struct Printer {
    /// usblp device node
    pub lp: PathBuf,
    /// usbfs device node
    pub usbfs: PathBuf,
    pub serial: String,
}

fn read_attr(dir: &Path, name: &str) -> Result<String> {
    let path = dir.join(name);
    Ok(fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?
        .trim()
        .to_string())
}

impl Printer {
    pub fn find_all() -> Result<Vec<Printer>> {
        let mut found = Vec::new();
        for entry in fs::read_dir("/sys/bus/usb/devices")? {
            let dir = entry?.path();
            if read_attr(&dir, "idVendor").ok().as_deref() != Some(VENDOR_ID)
                || read_attr(&dir, "idProduct").ok().as_deref() != Some(PRODUCT_ID)
            {
                continue;
            }
            let busnum: u32 = read_attr(&dir, "busnum")?.parse()?;
            let devnum: u32 = read_attr(&dir, "devnum")?.parse()?;
            let name = dir.file_name().unwrap().to_string_lossy().into_owned();
            let usbmisc = dir.join(format!("{name}:1.0/usbmisc"));
            let lp = fs::read_dir(&usbmisc)
                .ok()
                .and_then(|mut d| d.next())
                .and_then(|e| e.ok())
                .map(|e| Path::new("/dev/usb").join(e.file_name()))
                .ok_or_else(|| anyhow!("{name}: usblp is not bound (no {})", usbmisc.display()))?;
            found.push(Printer {
                lp,
                usbfs: PathBuf::from(format!("/dev/bus/usb/{busnum:03}/{devnum:03}")),
                serial: read_attr(&dir, "serial").unwrap_or_default(),
            });
        }
        Ok(found)
    }

    pub fn find() -> Result<Printer> {
        let mut all = Self::find_all()?;
        match all.len() {
            0 => bail!("no SR5500P found (USB {VENDOR_ID}:{PRODUCT_ID})"),
            1 => Ok(all.remove(0)),
            n => bail!("{n} printers found; unplug the others"),
        }
    }

    pub fn status(&self) -> Result<Status> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&self.usbfs)
            .with_context(|| format!("failed to open {}", self.usbfs.display()))?;
        let mut raw = [0u8; 8];
        let n = control_in(&file, 0xc1, 0x01, 0, 0, &mut raw)
            .context("failed to read printer status")?;
        if n != raw.len() {
            bail!("short status: {:02x?}", &raw[..n]);
        }
        Ok(Status { raw })
    }

    pub fn send(&self, data: &[u8]) -> Result<()> {
        let mut file = File::options()
            .write(true)
            .open(&self.lp)
            .with_context(|| format!("failed to open {}", self.lp.display()))?;
        file.write_all(data)?;
        file.flush()?;
        Ok(())
    }
}

#[repr(C)]
struct UsbCtrlTransfer {
    request_type: u8,
    request: u8,
    value: u16,
    index: u16,
    length: u16,
    timeout: u32,
    data: *mut libc::c_void,
}

// _IOWR('U', 0, struct usbdevfs_ctrltransfer)
const USBDEVFS_CONTROL: libc::c_ulong = (3 << 30)
    | ((size_of::<UsbCtrlTransfer>() as libc::c_ulong) << 16)
    | ((b'U' as libc::c_ulong) << 8);

fn control_in(file: &File, request_type: u8, request: u8, value: u16, index: u16, buf: &mut [u8]) -> Result<usize> {
    let mut xfer = UsbCtrlTransfer {
        request_type,
        request,
        value,
        index,
        length: buf.len() as u16,
        timeout: 1000,
        data: buf.as_mut_ptr().cast(),
    };
    // SAFETY: xfer points to a buffer of `length` bytes that outlives the call.
    let r = unsafe { libc::ioctl(file.as_raw_fd(), USBDEVFS_CONTROL as _, &mut xfer) };
    if r < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(r as usize)
}

/// 8-byte status block: `[len, state, error, tape width, tape kind, ...]`.
#[derive(Copy, Clone, Debug)]
pub struct Status {
    pub raw: [u8; 8],
}

impl Status {
    pub fn state(&self) -> u8 {
        self.raw[1]
    }

    pub fn error(&self) -> u8 {
        self.raw[2]
    }

    pub fn tape(&self) -> Option<Tape> {
        Tape::from_status_code(self.raw[3])
    }

    pub fn state_name(&self) -> &'static str {
        match self.state() {
            0 => "idle",
            1 => "feeding",
            2 => "printing",
            3 => "receiving data",
            4 => "feed end",
            5 => "print end",
            _ => "unknown",
        }
    }

    pub fn error_name(&self) -> &'static str {
        match self.error() {
            0x00 => "none",
            0x01 => "cutter error",
            0x06 => "no tape cartridge",
            0x15 => "head overheated",
            0x20 => "cancelled on the printer",
            0x21 => "cover open",
            0x22 => "low voltage",
            0x30 => "tape feed error",
            0x42 => "tape end",
            _ => "unknown",
        }
    }
}

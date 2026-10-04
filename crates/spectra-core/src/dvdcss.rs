//! The system's libdvdcss, for reading a DVD in the clear.
//!
//! Most films' discs are scrambled with CSS. Spectra carries no CSS code of
//! its own: the library VLC already plays them through is loaded when a copy
//! is made, and only then, so it stays the system's to install - and to
//! keep updated - or not. Each VOB file has its own title key; asking for it
//! at the file's first sector is enough for libdvdcss to decrypt everything
//! up to the next, and it clears each sector's scrambled flag as it goes, so
//! the copy is an ordinary DVD.

use std::ffi::{CString, c_char, c_int, c_void};
use std::path::Path;

use crate::copy::DvdSectors;
use crate::{Error, Result};

const BLOCK: usize = 2048;
const READ_DECRYPT: c_int = 1;
const SEEK_KEY: c_int = 1 << 1;

type Open = unsafe extern "C" fn(*const c_char) -> *mut c_void;
type Close = unsafe extern "C" fn(*mut c_void) -> c_int;
type Seek = unsafe extern "C" fn(*mut c_void, c_int, c_int) -> c_int;
type Read = unsafe extern "C" fn(*mut c_void, *mut c_void, c_int, c_int) -> c_int;

pub struct Dvdcss {
    library: *mut c_void,
    handle: *mut c_void,
    close: Close,
    seek: Seek,
    read: Read,
}

// SAFETY: one handle, used from one thread at a time through &mut self.
unsafe impl Send for Dvdcss {}

fn symbol<T>(library: *mut c_void, name: &str) -> Result<T> {
    let name = CString::new(name).expect("no NUL in a symbol name");
    // SAFETY: a valid library handle and C string.
    let found = unsafe { libc::dlsym(library, name.as_ptr()) };
    if found.is_null() {
        return Err(Error::Unsupported(format!("libdvdcss without {name:?}")));
    }
    // SAFETY: T is the function pointer type libdvdcss's header gives.
    Ok(unsafe { std::mem::transmute_copy(&found) })
}

impl Dvdcss {
    /// Open the drive (`/dev/srN`) through libdvdcss.
    pub fn open(drive: &Path) -> Result<Self> {
        let soname = c"libdvdcss.so.2";
        // SAFETY: a valid C string; the handle is closed in Drop.
        let library = unsafe { libc::dlopen(soname.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
        if library.is_null() {
            return Err(Error::Unsupported(
                "copying a DVD needs libdvdcss, which isn't installed".into(),
            ));
        }
        let close_library = || {
            // SAFETY: opened above, and nothing from it is used after this.
            unsafe { libc::dlclose(library) };
        };
        let open: Open = match symbol(library, "dvdcss_open") {
            Ok(f) => f,
            Err(e) => {
                close_library();
                return Err(e);
            }
        };
        let functions = (|| {
            Ok::<_, Error>((
                symbol::<Close>(library, "dvdcss_close")?,
                symbol::<Seek>(library, "dvdcss_seek")?,
                symbol::<Read>(library, "dvdcss_read")?,
            ))
        })();
        let (close, seek, read) = match functions {
            Ok(f) => f,
            Err(e) => {
                close_library();
                return Err(e);
            }
        };
        let path = CString::new(drive.as_os_str().as_encoded_bytes())
            .map_err(|_| Error::Unsupported("a drive path with a NUL in it".into()))?;
        // SAFETY: a valid C string.
        let handle = unsafe { open(path.as_ptr()) };
        if handle.is_null() {
            close_library();
            return Err(Error::Unsupported(format!(
                "libdvdcss could not open {}",
                drive.display()
            )));
        }
        Ok(Self {
            library,
            handle,
            close,
            seek,
            read,
        })
    }

    fn seek_to(&mut self, lba: u32, flags: c_int) -> Result<()> {
        let at =
            c_int::try_from(lba).map_err(|_| Error::Unsupported("a sector past 4 TB".into()))?;
        // SAFETY: an open handle.
        if unsafe { (self.seek)(self.handle, at, flags) } != at {
            return Err(Error::Unsupported(format!(
                "libdvdcss could not seek to {lba}"
            )));
        }
        Ok(())
    }
}

impl DvdSectors for Dvdcss {
    fn key(&mut self, lba: u32) -> Result<()> {
        self.seek_to(lba, SEEK_KEY)
            .map_err(|_| Error::Unsupported("the disc's keys could not be had".into()))
    }

    fn read(&mut self, lba: u32, count: u32, decrypt: bool) -> Result<Vec<u8>> {
        self.seek_to(lba, 0)?;
        let mut data = vec![0; count as usize * BLOCK];
        let flags = if decrypt { READ_DECRYPT } else { 0 };
        let blocks = c_int::try_from(count).unwrap_or(c_int::MAX);
        // SAFETY: an open handle, and a buffer of `count` blocks.
        let got = unsafe { (self.read)(self.handle, data.as_mut_ptr().cast(), blocks, flags) };
        if got != blocks {
            return Err(Error::Unsupported(format!(
                "libdvdcss could not read {lba}"
            )));
        }
        Ok(data)
    }
}

impl Drop for Dvdcss {
    fn drop(&mut self) {
        // SAFETY: opened in `open`, and closed only here.
        unsafe {
            (self.close)(self.handle);
            libc::dlclose(self.library);
        }
    }
}

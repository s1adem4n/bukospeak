use std::iter;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, DeleteObject, GetDC, GetDIBits, HBITMAP, HDC, ReleaseDC,
};
use windows::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW};
use windows::Win32::UI::Shell::SHDefExtractIconW;
use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, HICON, ICONINFO};
use windows::core::PCWSTR;

/// The width and height an icon is read at: a tile's on a display of three times the density.
const ICON: u32 = 72;

/// An icon's pixels, rows from the top, each pixel blue, green, red and alpha.
pub struct Icon {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

fn wide(text: &std::ffi::OsStr) -> Vec<u16> {
    text.encode_wide().chain(iter::once(0)).collect()
}

/// The description in the program's version information, which Windows shows as its name.
pub fn title(path: &Path) -> Option<String> {
    let path = wide(path.as_os_str());
    // SAFETY: `path` is a NUL-terminated UTF-16 string that outlives the call.
    let size = unsafe { GetFileVersionInfoSizeW(PCWSTR(path.as_ptr()), None) };
    if size == 0 {
        return None;
    }
    let mut info = vec![0_u8; size as usize];
    // SAFETY: `info` is as large as Windows said the version information is; `path` is as above.
    unsafe { GetFileVersionInfoW(PCWSTR(path.as_ptr()), None, size, info.as_mut_ptr().cast()) }.ok()?;
    // Pairs of a language and a code page, the first the one the file is in.
    let translation = query(&info, "\\VarFileInfo\\Translation", 1)?;
    let [language, code_page] = translation.as_chunks().0.first_chunk::<2>()?.map(u16::from_le_bytes);
    let description = query(&info, &format!("\\StringFileInfo\\{language:04x}{code_page:04x}\\FileDescription"), 2)?;
    let units: Vec<u16> = description.as_chunks().0.iter().map(|&unit| u16::from_le_bytes(unit)).take_while(|&unit| unit != 0).collect();
    let title = String::from_utf16_lossy(&units).trim().to_owned();
    (!title.is_empty()).then_some(title)
}

/// The value at `name` in the version information `info`, which Windows measures in units of
/// `unit` bytes.
fn query<'a>(info: &'a [u8], name: &str, unit: usize) -> Option<&'a [u8]> {
    let name = wide(std::ffi::OsStr::new(name));
    let mut value = std::ptr::null_mut();
    let mut length = 0_u32;
    // SAFETY: `info` is the version information GetFileVersionInfoW filled, `name` a NUL-terminated
    // UTF-16 string; the value found lies inside `info`.
    let found = unsafe { VerQueryValueW(info.as_ptr().cast(), PCWSTR(name.as_ptr()), &mut value, &mut length) }.as_bool();
    if !found || value.is_null() {
        return None;
    }
    let start = (value as usize).checked_sub(info.as_ptr() as usize)?;
    info.get(start..start.checked_add(length as usize * unit)?)
}

/// The program's icon, as the Explorer shows it.
pub fn icon(path: &Path) -> Option<Icon> {
    let path = wide(path.as_os_str());
    let mut icon = HICON::default();
    // SAFETY: `path` is a NUL-terminated UTF-16 string; the icon made is destroyed by its guard.
    unsafe { SHDefExtractIconW(PCWSTR(path.as_ptr()), 0, 0, Some(&mut icon), None, ICON) }.ok().ok()?;
    if icon.is_invalid() {
        return None;
    }
    let icon = OwnedIcon(icon);
    let mut info = ICONINFO::default();
    // SAFETY: the icon is valid; the bitmaps GetIconInfo copies are deleted by their guards.
    unsafe { GetIconInfo(icon.0, &mut info) }.ok()?;
    let (color, mask) = (OwnedBitmap(info.hbmColor), OwnedBitmap(info.hbmMask));
    if color.0.is_invalid() {
        return None;
    }
    let screen = Screen::get()?;
    let (width, height, mut bgra) = pixels(&screen, &color)?;
    // An icon without alpha is opaque where its mask is black, and transparent where it is white.
    if bgra.as_chunks::<4>().0.iter().all(|pixel| pixel[3] == 0) {
        let (_, _, transparent) = pixels(&screen, &mask)?;
        for (pixel, mask) in bgra.as_chunks_mut::<4>().0.iter_mut().zip(transparent.as_chunks::<4>().0) {
            pixel[3] = if mask[0] == 0 { u8::MAX } else { 0 };
        }
    }
    Some(Icon { width, height, bgra })
}

/// The bitmap's pixels, rows from the top, four bytes each.
fn pixels(screen: &Screen, bitmap: &OwnedBitmap) -> Option<(u32, u32, Vec<u8>)> {
    let mut format = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER { biSize: size_of::<BITMAPINFOHEADER>() as u32, ..BITMAPINFOHEADER::default() },
        ..BITMAPINFO::default()
    };
    // SAFETY: with no buffer, GetDIBits only fills in `format` with the bitmap's size.
    if unsafe { GetDIBits(screen.0, bitmap.0, 0, 0, None, &mut format, DIB_RGB_COLORS) } == 0 {
        return None;
    }
    let (width, height) = (format.bmiHeader.biWidth.unsigned_abs(), format.bmiHeader.biHeight.unsigned_abs());
    format.bmiHeader = BITMAPINFOHEADER {
        biSize: size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: width as i32,
        // Negative: rows from the top.
        biHeight: -(height as i32),
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB.0,
        ..BITMAPINFOHEADER::default()
    };
    let mut pixels = vec![0_u8; width as usize * height as usize * 4];
    // SAFETY: `pixels` holds `height` rows of `width` pixels of the 32 bits `format` asks for.
    let rows = unsafe { GetDIBits(screen.0, bitmap.0, 0, height, Some(pixels.as_mut_ptr().cast()), &mut format, DIB_RGB_COLORS) };
    (rows == height as i32).then_some((width, height, pixels))
}

struct OwnedIcon(HICON);

impl Drop for OwnedIcon {
    fn drop(&mut self) {
        // SAFETY: the icon was made for this guard alone and is destroyed once. A failure leaves
        // only a handle until the app quits.
        let _ = unsafe { DestroyIcon(self.0) };
    }
}

struct OwnedBitmap(HBITMAP);

impl Drop for OwnedBitmap {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            // SAFETY: the bitmap was copied for this guard alone and is deleted once. A failure
            // leaves only a handle until the app quits.
            let _ = unsafe { DeleteObject(self.0.into()) };
        }
    }
}

/// The screen's device context, which reads bitmaps in the screen's format.
struct Screen(HDC);

impl Screen {
    fn get() -> Option<Self> {
        // SAFETY: GetDC with no window takes the screen's context, which the guard gives back.
        let context = unsafe { GetDC(None) };
        (!context.is_invalid()).then_some(Self(context))
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        // SAFETY: the context was taken by this guard and is given back once. It only says
        // whether it was released; one not released stays until the app quits.
        let _ = unsafe { ReleaseDC(None, self.0) };
    }
}

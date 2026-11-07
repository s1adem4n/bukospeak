use crate::{Architecture, Error};

#[cfg(windows)]
pub fn detect() -> Result<Option<Architecture>, Error> {
    use std::ffi::{CStr, c_char};

    use libloading::os::windows::{LOAD_LIBRARY_SEARCH_SYSTEM32, Library};

    const CUDA_ERROR_NO_DEVICE: i32 = 100;
    const COMPUTE_CAPABILITY_MAJOR: i32 = 75;
    const COMPUTE_CAPABILITY_MINOR: i32 = 76;

    // SAFETY: the driver's CUDA library does nothing unsound on loading; it is loaded only from
    // the system folder, where the driver puts it.
    let Ok(cuda) = (unsafe { Library::load_with_flags("nvcuda.dll", LOAD_LIBRARY_SEARCH_SYSTEM32) }) else {
        // NVIDIA's driver isn't installed.
        return Ok(None);
    };
    let check = |call: &'static str, code: i32| if code == 0 { Ok(()) } else { Err(Error::Cuda { call, code }) };
    // SAFETY: each type is as `cuda.h` declares it, and the library stays loaded past the last call.
    let (init, device_get, attribute, name) = unsafe {
        (
            *cuda.get::<unsafe extern "C" fn(u32) -> i32>(c"cuInit").map_err(|source| Error::Symbol { name: c"cuInit", source })?,
            *cuda
                .get::<unsafe extern "C" fn(*mut i32, i32) -> i32>(c"cuDeviceGet")
                .map_err(|source| Error::Symbol { name: c"cuDeviceGet", source })?,
            *cuda
                .get::<unsafe extern "C" fn(*mut i32, i32, i32) -> i32>(c"cuDeviceGetAttribute")
                .map_err(|source| Error::Symbol { name: c"cuDeviceGetAttribute", source })?,
            *cuda
                .get::<unsafe extern "C" fn(*mut c_char, i32, i32) -> i32>(c"cuDeviceGetName")
                .map_err(|source| Error::Symbol { name: c"cuDeviceGetName", source })?,
        )
    };
    // SAFETY: no flags, as CUDA asks.
    match unsafe { init(0) } {
        CUDA_ERROR_NO_DEVICE => return Ok(None),
        code => check("cuInit", code)?,
    }
    let mut device = 0;
    // SAFETY: CUDA writes the first device where the pointer points.
    check("cuDeviceGet", unsafe { device_get(&mut device, 0) })?;
    let (mut major, mut minor) = (0, 0);
    // SAFETY: CUDA writes each attribute of the device where its pointer points.
    check("cuDeviceGetAttribute", unsafe { attribute(&mut major, COMPUTE_CAPABILITY_MAJOR, device) })?;
    // SAFETY: as above.
    check("cuDeviceGetAttribute", unsafe { attribute(&mut minor, COMPUTE_CAPABILITY_MINOR, device) })?;
    let mut buffer = [0 as c_char; 256];
    // SAFETY: CUDA writes the device's name, nul-terminated, into no more than the buffer's length.
    check("cuDeviceGetName", unsafe { name(buffer.as_mut_ptr(), buffer.len() as i32, device) })?;
    // SAFETY: CUDA terminated the name within the buffer.
    let name = unsafe { CStr::from_ptr(buffer.as_ptr()) }.to_string_lossy();
    // GeForce GTX 16 cards are Turing too, but lack the Tensor Cores the effects need.
    Ok(Architecture::of_compute_capability(major, minor).filter(|_| name.contains("RTX")))
}

#[cfg(not(windows))]
pub fn detect() -> Result<Option<Architecture>, Error> {
    Ok(None)
}

//! NVIDIA's runtime, loaded, and the effects it runs, through the C interface
//! `nvAudioEffects.h` declares.

use std::ffi::{CStr, CString, c_char, c_void};
use std::fmt;
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::Arc;

use libloading::Library;

use crate::{Architecture, Error, FRAME, Kind, SAMPLE_RATE};

/// How much the runtime tells on standard error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Fatal = 0,
    Error = 1,
    Warning = 2,
    Info = 3,
}

/// What a call into the runtime returned, if not success.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Status(i32);

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self.0 {
            1 => "failed",
            2 => "invalid handle",
            3 => "invalid parameter",
            4 => "immutable parameter",
            5 => "insufficient data",
            6 => "effect not available",
            7 => "output buffer too small",
            8 => "model load failed",
            9 => "model not loaded",
            10 => "incompatible model",
            11 => "GPU unsupported",
            12 => "no supported GPU found",
            13 => "wrong GPU",
            14 => "CUDA error",
            15 => "invalid operation",
            16 => "unsupported runtime",
            19 => "CUDA context creation failed",
            20 => "library error",
            21 => "out of memory",
            _ => return write!(f, "status {}", self.0),
        };
        f.write_str(name)
    }
}

type Handle = *mut c_void;

/// The runtime's functions the crate calls, as `nvAudioEffects.h` declares them.
#[derive(Clone, Copy)]
struct Api {
    create_effect: unsafe extern "C" fn(*const c_char, *mut Handle) -> i32,
    destroy_effect: unsafe extern "C" fn(Handle) -> i32,
    set_u32: unsafe extern "C" fn(Handle, *const c_char, u32) -> i32,
    set_string: unsafe extern "C" fn(Handle, *const c_char, *const c_char) -> i32,
    get_u32: unsafe extern "C" fn(Handle, *const c_char, *mut u32) -> i32,
    get_u32_list: unsafe extern "C" fn(Handle, *const c_char, *mut u32, *mut u32) -> i32,
    load: unsafe extern "C" fn(Handle) -> i32,
    run: unsafe extern "C" fn(Handle, *const *const f32, *const *mut f32, u32, u32) -> i32,
    reset: unsafe extern "C" fn(Handle) -> i32,
    initialize_logger: unsafe extern "C" fn(i32, i32, *const c_char, *const c_void, *mut c_void) -> i32,
}

fn check(call: &'static str, status: i32) -> Result<(), Error> {
    match status {
        0 => Ok(()),
        status => Err(Error::Call { call, status: Status(status) }),
    }
}

/// NVIDIA's runtime, loaded.
pub struct Runtime {
    api: Api,
    root: PathBuf,
    // The runtime and the libraries it uses, unloaded in the reverse of the order loaded.
    libraries: Vec<Library>,
}

impl Drop for Runtime {
    fn drop(&mut self) {
        while let Some(library) = self.libraries.pop() {
            drop(library);
        }
    }
}

impl Runtime {
    /// Loads the runtime from the download at `root`.
    #[cfg(windows)]
    pub fn load(root: &Path) -> Result<Self, Error> {
        use libloading::os::windows::{LOAD_WITH_ALTERED_SEARCH_PATH, Library as WindowsLibrary};

        let libraries = crate::RUNTIME
            .iter()
            .map(|file| {
                let path = root.join(file);
                // SAFETY: NVIDIA's libraries do nothing on loading that could break the process's
                // invariants. Each is loaded by its full path with the altered search path, which
                // finds what it uses beside it first, and after those it uses, so those the
                // runtime and the effects load by name later are found loaded already.
                unsafe { WindowsLibrary::load_with_flags(&path, LOAD_WITH_ALTERED_SEARCH_PATH) }
                    .map(Library::from)
                    .map_err(|source| Error::Library { path, source })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let runtime = libraries.last().expect("the runtime is a library");
        // SAFETY: each type is the function's as `nvAudioEffects.h` declares it. The pointers
        // stay valid while the runtime is loaded, which `Runtime` keeps it while it holds them.
        let api = unsafe {
            Api {
                create_effect: function(runtime, c"NvAFX_CreateEffect")?,
                destroy_effect: function(runtime, c"NvAFX_DestroyEffect")?,
                set_u32: function(runtime, c"NvAFX_SetU32")?,
                set_string: function(runtime, c"NvAFX_SetString")?,
                get_u32: function(runtime, c"NvAFX_GetU32")?,
                get_u32_list: function(runtime, c"NvAFX_GetU32List")?,
                load: function(runtime, c"NvAFX_Load")?,
                run: function(runtime, c"NvAFX_Run")?,
                reset: function(runtime, c"NvAFX_Reset")?,
                initialize_logger: function(runtime, c"NvAFX_InitializeLogger")?,
            }
        };
        Ok(Self { api, root: root.to_owned(), libraries })
    }

    /// Loads the runtime, which only Windows has.
    #[cfg(not(windows))]
    pub fn load(_root: &Path) -> Result<Self, Error> {
        Err(Error::Unsupported)
    }

    /// Has the runtime tell on standard error what is as severe as `severity` or more.
    pub fn log(&self, severity: Severity) -> Result<(), Error> {
        const STANDARD_ERROR: i32 = 0;
        // SAFETY: with standard error as the target, the runtime reads neither the file's name
        // nor the callback, which may be null.
        check("NvAFX_InitializeLogger", unsafe {
            (self.api.initialize_logger)(severity as i32, STANDARD_ERROR, ptr::null(), ptr::null(), ptr::null_mut())
        })
    }
}

/// The function `name` of `library`.
///
/// # Safety
///
/// `T` is the function's type, and the pointer is called only while `library` is loaded.
#[cfg(windows)]
unsafe fn function<T: Copy>(library: &Library, name: &'static CStr) -> Result<T, Error> {
    // SAFETY: the caller vouches for the type.
    unsafe { library.get::<T>(name) }.map(|symbol| *symbol).map_err(|source| Error::Symbol { name, source })
}

/// An effect, loaded on the GPU, taking and giving frames of [`FRAME`] samples at
/// [`SAMPLE_RATE`].
pub struct Effect {
    runtime: Arc<Runtime>,
    handle: Handle,
}

// SAFETY: the runtime ties an effect to no thread: it makes the effect's CUDA context current
// for each call and pops it after, so the effect may be loaded on one thread and run on another,
// as measured. `&mut self` keeps its calls one at a time.
unsafe impl Send for Effect {}

impl Effect {
    /// Loads the effect `kind` with the models for `architecture`. Building the model on the
    /// GPU takes up to seconds.
    pub fn new(runtime: &Arc<Runtime>, kind: Kind, architecture: Architecture) -> Result<Self, Error> {
        let mut handle = ptr::null_mut();
        let selector = CString::new(kind.name()).expect("an effect's name holds no nul");
        // SAFETY: the selector is a C string, and the runtime writes the handle where it points.
        check("NvAFX_CreateEffect", unsafe { (runtime.api.create_effect)(selector.as_ptr(), &mut handle) })?;
        // Destroyed when an error returns from here on.
        let mut effect = Self { runtime: Arc::clone(runtime), handle };
        let model = runtime.root.join("features").join(kind.feature()).join("models").join(architecture.name()).join(kind.model());
        effect.set_u32(c"input_sample_rate", SAMPLE_RATE)?;
        effect.set_path(c"model_path", &model)?;
        let supported = effect.u32_list(c"supported_num_samples_per_frame")?;
        match supported[..] {
            // An effect that takes frames of one size only refuses to be told it.
            [one] if one as usize == FRAME => {}
            _ if supported.contains(&(FRAME as u32)) => effect.set_u32(c"num_samples_per_input_frame", FRAME as u32)?,
            _ => return Err(Error::Frame { supported }),
        }
        // SAFETY: the handle is the effect's, created above.
        check("NvAFX_Load", unsafe { (runtime.api.load)(effect.handle) })?;
        let frames = (effect.u32(c"num_samples_per_input_frame")?, effect.u32(c"num_samples_per_output_frame")?);
        if frames != (FRAME as u32, FRAME as u32) {
            return Err(Error::Frame { supported: vec![frames.0, frames.1] });
        }
        Ok(effect)
    }

    /// Runs the effect on the next frame, in −1.0..=1.0, into `output`.
    pub fn run(&mut self, input: &[f32; FRAME], output: &mut [f32; FRAME]) -> Result<(), Error> {
        let (input, output) = (input.as_ptr(), output.as_mut_ptr());
        // SAFETY: one channel each way, of the frame the effect was loaded to take and give,
        // which the runtime reads and writes no further than.
        check("NvAFX_Run", unsafe { (self.runtime.api.run)(self.handle, &input, &output, FRAME as u32, 1) })
    }

    /// Starts anew, as after the input stopped: nothing from before comes out after.
    pub fn reset(&mut self) -> Result<(), Error> {
        // SAFETY: the handle is the effect's, valid until it drops.
        check("NvAFX_Reset", unsafe { (self.runtime.api.reset)(self.handle) })
    }

    fn set_u32(&mut self, name: &CStr, value: u32) -> Result<(), Error> {
        // SAFETY: the handle is the effect's, and the name a C string.
        check("NvAFX_SetU32", unsafe { (self.runtime.api.set_u32)(self.handle, name.as_ptr(), value) })
    }

    fn set_path(&mut self, name: &CStr, path: &Path) -> Result<(), Error> {
        let value = path.to_str().and_then(|path| CString::new(path).ok()).ok_or_else(|| Error::Path(path.to_owned()))?;
        // SAFETY: the handle is the effect's, and the name and the value C strings.
        check("NvAFX_SetString", unsafe { (self.runtime.api.set_string)(self.handle, name.as_ptr(), value.as_ptr()) })
    }

    fn u32(&self, name: &CStr) -> Result<u32, Error> {
        let mut value = 0;
        // SAFETY: the handle is the effect's, the name a C string, and the runtime writes one
        // value where the pointer points.
        check("NvAFX_GetU32", unsafe { (self.runtime.api.get_u32)(self.handle, name.as_ptr(), &mut value) })?;
        Ok(value)
    }

    fn u32_list(&self, name: &CStr) -> Result<Vec<u32>, Error> {
        let mut list = vec![0; 64];
        let mut size = list.len() as u32;
        // SAFETY: the handle is the effect's and the name a C string; the runtime writes no more
        // values than `size` says the list holds, and then how many it wrote.
        check("NvAFX_GetU32List", unsafe { (self.runtime.api.get_u32_list)(self.handle, name.as_ptr(), list.as_mut_ptr(), &mut size) })?;
        list.truncate(size as usize);
        Ok(list)
    }
}

impl Drop for Effect {
    fn drop(&mut self) {
        // SAFETY: the handle is the effect's, and isn't used after.
        let status = unsafe { (self.runtime.api.destroy_effect)(self.handle) };
        // The effect is gone either way; a failure to free it leaves nothing to do.
        debug_assert_eq!(status, 0, "NvAFX_DestroyEffect: {}", Status(status));
    }
}

#[cfg(all(test, not(windows)))]
mod tests {
    use super::*;

    #[test]
    fn the_runtime_loads_only_on_windows() {
        assert!(matches!(Runtime::load(Path::new(".")), Err(Error::Unsupported)));
    }
}

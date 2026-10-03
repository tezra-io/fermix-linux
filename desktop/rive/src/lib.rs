//! Plays a Rive animation offscreen and hands each frame back as pixels.
//!
//! It is the Rive C++ runtime (`vendor/`, pinned by `scripts/vendor_rive.sh`)
//! and its GL renderer, in an EGL context of its own, behind the shim in
//! `shim/fermix_rive.h`. It knows nothing of GTK or of the mascot: the caller
//! names the state machine and the properties it writes. Every call that
//! touches GL puts back whatever context the thread had before, so a toolkit's
//! own GL on the same thread is left as it was.

use std::ffi::{c_char, CStr, CString};
use std::fmt;
use std::marker::PhantomData;
use std::ptr::NonNull;
use std::rc::Rc;

/// Room for the shim's one-sentence errors.
const ERROR_LEN: usize = 256;

mod ffi {
    use std::ffi::c_char;

    #[repr(C)]
    pub struct FxStage {
        _private: [u8; 0],
    }

    #[repr(C)]
    pub struct FxScene {
        _private: [u8; 0],
    }

    extern "C" {
        pub fn fx_stage_new(
            riv: *const u8,
            riv_len: usize,
            error: *mut c_char,
            error_len: usize,
        ) -> *mut FxStage;
        pub fn fx_stage_free(stage: *mut FxStage);
        pub fn fx_scene_new(
            stage: *mut FxStage,
            state_machine: *const c_char,
            error: *mut c_char,
            error_len: usize,
        ) -> *mut FxScene;
        pub fn fx_scene_free(stage: *mut FxStage, scene: *mut FxScene);
        pub fn fx_scene_set_enum(
            scene: *mut FxScene,
            property: *const c_char,
            value: *const c_char,
        ) -> bool;
        pub fn fx_scene_set_number(
            scene: *mut FxScene,
            property: *const c_char,
            value: f32,
        ) -> bool;
        pub fn fx_scene_set_boolean(
            scene: *mut FxScene,
            property: *const c_char,
            value: bool,
        ) -> bool;
        pub fn fx_scene_advance(scene: *mut FxScene, seconds: f32);
        pub fn fx_scene_render(
            stage: *mut FxStage,
            scene: *mut FxScene,
            width: u32,
            height: u32,
            rgba: *mut u8,
            rgba_len: usize,
            error: *mut c_char,
            error_len: usize,
        ) -> bool;
    }
}

/// Why the runtime could not do what was asked, in one sentence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

/// An offscreen GL context with the Rive renderer, and one `.riv` file in it.
/// It stays on the thread that made it: GL contexts are current per thread.
pub struct Stage {
    raw: NonNull<ffi::FxStage>,
    _thread_bound: PhantomData<*const ()>,
}

impl Stage {
    /// Opens the context and imports `riv`, which must be a Rive runtime file.
    pub fn new(riv: &[u8]) -> Result<Rc<Stage>, Error> {
        if riv.is_empty() {
            return Err(Error("the animation file is empty".into()));
        }
        let mut error = [0 as c_char; ERROR_LEN];
        // SAFETY: `riv` is valid for its length and the shim copies it; the
        // error buffer is valid for ERROR_LEN.
        let raw =
            unsafe { ffi::fx_stage_new(riv.as_ptr(), riv.len(), error.as_mut_ptr(), ERROR_LEN) };
        match NonNull::new(raw) {
            Some(raw) => Ok(Rc::new(Stage {
                raw,
                _thread_bound: PhantomData,
            })),
            None => Err(sentence(&error)),
        }
    }
}

impl Drop for Stage {
    fn drop(&mut self) {
        // SAFETY: the stage was made by fx_stage_new and every scene holds an
        // Rc to it, so none outlives it.
        unsafe { ffi::fx_stage_free(self.raw.as_ptr()) }
    }
}

/// One artboard of the stage's file, its state machine, and the view model
/// instance bound to it.
pub struct Scene {
    raw: NonNull<ffi::FxScene>,
    stage: Rc<Stage>,
}

/// One drawn frame: premultiplied RGBA, 8 bits a channel, top row first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Frame {
    /// Bytes from one row to the next.
    pub fn stride(&self) -> usize {
        self.width as usize * 4
    }
}

impl Scene {
    /// The file's default artboard, playing `state_machine`, bound to the
    /// artboard's default view model instance. The state machine has not run:
    /// write what it reads at its start, then [`Scene::start`] it.
    pub fn new(stage: &Rc<Stage>, state_machine: &str) -> Result<Scene, Error> {
        let name = c_string(state_machine);
        let mut error = [0 as c_char; ERROR_LEN];
        // SAFETY: the stage is alive for the call, the name is NUL-terminated,
        // and the error buffer is valid for ERROR_LEN.
        let raw = unsafe {
            ffi::fx_scene_new(
                stage.raw.as_ptr(),
                name.as_ptr(),
                error.as_mut_ptr(),
                ERROR_LEN,
            )
        };
        match NonNull::new(raw) {
            Some(raw) => Ok(Scene {
                raw,
                stage: Rc::clone(stage),
            }),
            None => Err(sentence(&error)),
        }
    }

    /// Writes an enum property of the view model instance.
    pub fn set_enum(&mut self, property: &str, value: &str) -> Result<(), Error> {
        let (name, word) = (c_string(property), c_string(value));
        // SAFETY: the scene is alive and both strings are NUL-terminated.
        let written =
            unsafe { ffi::fx_scene_set_enum(self.raw.as_ptr(), name.as_ptr(), word.as_ptr()) };
        if written {
            return Ok(());
        }
        Err(Error(format!(
            "the animation has no enum {property} with the value {value}"
        )))
    }

    /// Writes a number property of the view model instance.
    pub fn set_number(&mut self, property: &str, value: f32) -> Result<(), Error> {
        assert!(value.is_finite(), "{property} must be finite, got {value}");
        let name = c_string(property);
        // SAFETY: the scene is alive and the name is NUL-terminated.
        let written = unsafe { ffi::fx_scene_set_number(self.raw.as_ptr(), name.as_ptr(), value) };
        if written {
            return Ok(());
        }
        Err(Error(format!("the animation has no number {property}")))
    }

    /// Writes a boolean property of the view model instance.
    pub fn set_boolean(&mut self, property: &str, value: bool) -> Result<(), Error> {
        let name = c_string(property);
        // SAFETY: the scene is alive and the name is NUL-terminated.
        let written = unsafe { ffi::fx_scene_set_boolean(self.raw.as_ptr(), name.as_ptr(), value) };
        if written {
            return Ok(());
        }
        Err(Error(format!("the animation has no boolean {property}")))
    }

    /// Starts the state machine: one advance of nothing, in which it reads the
    /// properties written so far and enters its first states, then `lead`
    /// seconds in one advance. A first advance longer than nothing lands
    /// elsewhere than the same time played frame by frame.
    pub fn start(&mut self, lead: f32) {
        assert!(
            lead.is_finite() && lead >= 0.0,
            "a scene starts with a finite, non-negative lead, got {lead}"
        );
        self.advance(0.0);
        if lead > 0.0 {
            self.advance(lead);
        }
    }

    /// Advances the state machine by `seconds` and applies it to the artboard.
    pub fn advance(&mut self, seconds: f32) {
        assert!(
            seconds.is_finite() && seconds >= 0.0,
            "an animation advances by a finite, non-negative time, got {seconds}"
        );
        // SAFETY: the scene is alive.
        unsafe { ffi::fx_scene_advance(self.raw.as_ptr(), seconds) }
    }

    /// Draws the artboard fitted inside `width` by `height`, centred.
    pub fn render(&mut self, width: u32, height: u32) -> Result<Frame, Error> {
        assert!(
            width > 0 && height > 0,
            "a frame needs a size, got {width}x{height}"
        );
        let mut rgba = vec![0u8; width as usize * height as usize * 4];
        let mut error = [0 as c_char; ERROR_LEN];
        // SAFETY: the stage and scene are alive, `rgba` holds exactly
        // width * height * 4 bytes, and the error buffer is valid for ERROR_LEN.
        let drawn = unsafe {
            ffi::fx_scene_render(
                self.stage.raw.as_ptr(),
                self.raw.as_ptr(),
                width,
                height,
                rgba.as_mut_ptr(),
                rgba.len(),
                error.as_mut_ptr(),
                ERROR_LEN,
            )
        };
        if !drawn {
            return Err(sentence(&error));
        }
        Ok(Frame {
            width,
            height,
            rgba,
        })
    }
}

impl Drop for Scene {
    fn drop(&mut self) {
        // SAFETY: the scene was made by fx_scene_new on this stage, which the
        // Rc keeps alive until after this call.
        unsafe { ffi::fx_scene_free(self.stage.raw.as_ptr(), self.raw.as_ptr()) }
    }
}

/// A name for the shim. The names are the caller's own constants, so one with
/// a NUL in it is a programming error.
fn c_string(text: &str) -> CString {
    CString::new(text).unwrap_or_else(|_| panic!("{text:?} has a NUL in it"))
}

/// The sentence the shim wrote into `error`.
fn sentence(error: &[c_char; ERROR_LEN]) -> Error {
    // SAFETY: the shim always NUL-terminates what it writes, and the buffer
    // starts zeroed, so a NUL lies within it either way.
    let text = unsafe { CStr::from_ptr(error.as_ptr()) };
    let text = text.to_string_lossy().into_owned();
    assert!(!text.is_empty(), "the Rive shim failed without saying why");
    Error(text)
}

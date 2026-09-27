//! Compiles the vendored Rive runtime (`vendor/`, pinned by `scripts/vendor_rive.sh`)
//! and the shim that gives it a C interface, and links the system's EGL and libpng.
//!
//! Four static libraries, linked in dependency order: the shim, the GL renderer
//! with the PNG decoder, the runtime, then the GL loader. The vendored code is
//! compiled as shipped, with its own warnings off; the shim is compiled with
//! warnings as errors.

use std::path::{Path, PathBuf};

/// Every file in the runtime and the shim is built with these, because Rive's
/// class layouts follow them: `NDEBUG` in particular removes debug-only members.
const DEFINES: [(&str, Option<&str>); 3] = [
    ("NDEBUG", None),
    ("RIVE_DESKTOP_GL", None),
    ("RIVE_RAW_SHADERS", None),
];

/// The GL renderer's sources, as the runtime's own premake lists them for Linux.
const RENDERER_GL: [&str; 10] = [
    "renderer/src/gl/gl_state.cpp",
    "renderer/src/gl/gl_utils.cpp",
    "renderer/src/gl/load_store_actions_ext.cpp",
    "renderer/src/gl/render_buffer_gl_impl.cpp",
    "renderer/src/gl/render_context_gl_impl.cpp",
    "renderer/src/gl/render_target_gl.cpp",
    "renderer/src/gl/pls_impl_webgl.cpp",
    "renderer/src/gl/pls_impl_rw_texture.cpp",
    "renderer/src/ore/ore_binding_map.cpp",
    "renderer/src/ore/ore_bind_group_layout.cpp",
];

const DECODERS: [&str; 3] = [
    "decoders/src/bitmap_decoder.cpp",
    "decoders/src/bitmap_decoder_thirdparty.cpp",
    "decoders/src/decode_png.cpp",
];

const GLAD: [&str; 3] = [
    "renderer/glad/src/egl.c",
    "renderer/glad/src/gles2.c",
    "renderer/glad/glad_custom.c",
];

/// The runtime's source tree is a few levels deep; this bounds the walk.
const MAX_DEPTH: usize = 8;

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let runtime = root.join("vendor/rive-runtime");
    assert!(
        runtime.join("src").is_dir(),
        "{} is missing: run scripts/vendor_rive.sh",
        runtime.display()
    );
    println!("cargo:rerun-if-changed=vendor");
    println!("cargo:rerun-if-changed=shim");

    shim(&root, &runtime);
    renderer(&root, &runtime);
    core(&runtime);
    glad(&runtime);
    println!("cargo:rustc-link-lib=png");
    println!("cargo:rustc-link-lib=EGL");
}

/// A C++ build of the vendored code: C++17, optimised in every profile (the
/// animation is drawn 30 times a second), the runtime's defines, no warnings.
fn vendored() -> cc::Build {
    let mut build = cc::Build::new();
    build
        .cpp(true)
        .std("c++17")
        .opt_level(2)
        .warnings(false)
        .flag("-w");
    for (name, value) in DEFINES {
        build.define(name, value);
    }
    build
}

fn shim(root: &Path, runtime: &Path) {
    let mut build = cc::Build::new();
    build
        .cpp(true)
        .std("c++17")
        .opt_level(2)
        .warnings(true)
        .extra_warnings(true)
        .warnings_into_errors(true)
        .file(root.join("shim/fermix_rive.cpp"))
        // GCC instantiates rcp<Texture>'s destructor where the renderer's
        // headers only declare Texture; clang does not.
        .flag("-include")
        .flag("rive/renderer/texture.hpp");
    // The runtime's headers as system headers: its own warnings are not the shim's.
    for dir in [
        "include",
        "renderer/include",
        "renderer/glad",
        "renderer/glad/include",
    ] {
        build
            .flag("-isystem")
            .flag(runtime.join(dir).to_str().expect("a UTF-8 path"));
    }
    for (name, value) in DEFINES {
        build.define(name, value);
    }
    build.compile("fermix_rive_shim");
}

fn renderer(root: &Path, runtime: &Path) {
    let top = files_in(&runtime.join("renderer/src"), "cpp");
    let mut build = vendored();
    build
        .files(top)
        .files(RENDERER_GL.map(|f| runtime.join(f)))
        .files(DECODERS.map(|f| runtime.join(f)))
        .include(runtime.join("include"))
        .include(runtime.join("renderer/include"))
        .include(runtime.join("renderer/glad"))
        .include(runtime.join("renderer/glad/include"))
        .include(runtime.join("renderer/src"))
        .include(runtime.join("decoders/include"))
        .include(root.join("vendor"))
        .define("RIVE_PNG", None)
        .flag("-include")
        .flag("rive/renderer/texture.hpp");
    build.compile("rive_renderer");
}

fn core(runtime: &Path) {
    let mut sources = Vec::new();
    walk(&runtime.join("src"), 0, &mut sources);
    assert!(!sources.is_empty(), "the runtime has no sources");
    let mut build = vendored();
    build
        .files(sources)
        .include(runtime.join("include"))
        .define("_RIVE_INTERNAL_", None);
    build.compile("rive_runtime");
}

fn glad(runtime: &Path) {
    let mut build = cc::Build::new();
    build
        .opt_level(2)
        .warnings(false)
        .flag("-w")
        .files(GLAD.map(|f| runtime.join(f)))
        .include(runtime.join("renderer/glad"))
        .include(runtime.join("renderer/glad/include"));
    for (name, value) in DEFINES {
        build.define(name, value);
    }
    build.compile("rive_glad");
}

/// The files directly in `dir` with `extension`, in a stable order.
fn files_in(dir: &Path, extension: &str) -> Vec<PathBuf> {
    let entries = std::fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
    let mut files: Vec<PathBuf> = entries
        .map(|entry| entry.expect("a directory entry").path())
        .filter(|path| path.is_file() && path.extension().is_some_and(|e| e == extension))
        .collect();
    files.sort();
    files
}

/// Every `.cpp` under `dir`, depth first.
fn walk(dir: &Path, depth: usize, sources: &mut Vec<PathBuf>) {
    assert!(depth <= MAX_DEPTH, "{} is nested too deep", dir.display());
    sources.extend(files_in(dir, "cpp"));
    let entries = std::fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
    let mut dirs: Vec<PathBuf> = entries
        .map(|entry| entry.expect("a directory entry").path())
        .filter(|path| path.is_dir())
        .collect();
    dirs.sort();
    for sub in dirs {
        walk(&sub, depth + 1, sources);
    }
}

//! Compiles the app's icons and stylesheet into one GResource bundle.

fn main() {
    glib_build_tools::compile_resources(
        &["resources"],
        "resources/resources.gresource.xml",
        "fermix.gresource",
    );
}

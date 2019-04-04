
extern crate cc;


#[cfg(target_os = "macos")]
fn main() {
    cc::Build::new()
        .file("src/thorin.c")
        .file("src/mig/mach_excServer.c")
        .file("src/mig/mach_excUser.c")
        .flag("-Wno-unused-parameter")
        .flag("-Wno-unused-function")
        .compile("thorin");
}

#[cfg(target_os = "linux")]
fn main() {
    cc::Build::new()
        .file("src/thorin.c")
        .flag("-Wno-unused-parameter")
        .flag("-Wno-unused-function")
        .compile("thorin");
}

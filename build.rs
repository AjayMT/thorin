
extern crate cc;


fn main() {
    cc::Build::new()
        .file("src/thorin.c")
        .file("src/mig/mach_excServer.c")
        .file("src/mig/mach_excUser.c")
        .flag("-Wno-unused-parameter")
        .flag("-Wno-unused-function")
        .compile("thorin");
}

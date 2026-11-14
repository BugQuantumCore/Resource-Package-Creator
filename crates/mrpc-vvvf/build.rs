fn main() {
    println!("cargo:rerun-if-changed=cpp/vvvf.cpp");
    println!("cargo:rerun-if-changed=cpp/vvvf.h");
    let mut build = cc::Build::new();
    build
        .cpp(true)
        .opt_level(3)
        .flag_if_supported("-std=c++17")
        .warnings(true)
        .file("cpp/vvvf.cpp")
        .compile("mrpcvvvf");
}

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const RAPIDSNARK_DOWNLOAD_SCRIPT: &str = include_str!("./download_rapidsnark.sh");

const LIB_DIR_ENV: &str = "RAPIDSNARK_LIB_DIR";
const BASE_URL_ENV: &str = "RAPIDSNARK_DOWNLOAD_BASE_URL";

fn main() {
    println!("cargo:rerun-if-env-changed={LIB_DIR_ENV}");
    println!("cargo:rerun-if-env-changed={BASE_URL_ENV}");

    let target = env::var("TARGET").unwrap();
    let out_dir = env::var("OUT_DIR").expect("OUT_DIR not set");

    // There is no shared rapidsnark library for Windows.
    if is_windows_target() && !is_static_rapidsnark() {
        panic!("Windows needs the static-rapidsnark feature.");
    }

    // See: https://github.com/zkmopro/chkstk_stub
    chkstk_stub::build();

    let lib_path = match env::var_os(LIB_DIR_ENV) {
        Some(dir) => PathBuf::from(dir),
        None => download_prebuilt(&out_dir, &target),
    };

    let compiler = cc::Build::new().get_compiler();
    let cpp_stdlib = if compiler.is_like_clang() {
        "c++"
    } else {
        "stdc++"
    };

    println!("cargo:rustc-link-search=native={}", lib_path.display());

    // The shared rapidsnark artifact is already linked with its fr/fq/gmp implementation.
    // Linking those helper libraries again is redundant and can accidentally pull static archives
    // into downstream binaries, exporting generic Fr_* / Fq_* symbols that collide with other
    // native ZK libraries.
    //
    // The static rapidsnark archive is different: it leaves Fr_*, Fq_*, and GMP symbols unresolved,
    // so static/mobile builds must link the helper archives.
    if is_static_rapidsnark() || is_mobile_target() {
        println!("cargo:rustc-link-lib=static=rapidsnark");
        println!("cargo:rustc-link-lib=static=fr");
        println!("cargo:rustc-link-lib=static=fq");
        println!("cargo:rustc-link-lib=static=gmp");
    } else {
        println!("cargo:rustc-link-lib=dylib=rapidsnark");
    }

    if is_windows_target() {
        // Link the C++ runtime statically, so binaries don't need libstdc++-6.dll.
        println!("cargo:rustc-link-lib=static:-bundle={cpp_stdlib}");
        return;
    }

    println!("cargo:rustc-link-lib={cpp_stdlib}");

    // Android bundles pthread into libc
    let thread_lib = if target.contains("android") {
        "c"
    } else {
        "pthread"
    };
    println!("cargo:rustc-link-lib={thread_lib}");
}

fn is_static_rapidsnark() -> bool {
    env::var_os("CARGO_FEATURE_STATIC_RAPIDSNARK").is_some()
}

fn is_windows_target() -> bool {
    env::var("CARGO_CFG_TARGET_OS").unwrap() == "windows"
}

fn is_mobile_target() -> bool {
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap();
    target_os.contains("ios") || target_os.contains("android")
}

fn download_prebuilt(out_dir: &str, target: &str) -> PathBuf {
    let lib_path = Path::new(out_dir).join("rapidsnark").join(target);

    let script_path = Path::new(out_dir).join("download_rapidsnark.sh");
    fs::write(&script_path, RAPIDSNARK_DOWNLOAD_SCRIPT).expect("Failed to write build script");

    let status = Command::new("sh")
        .arg(&script_path)
        .env(
            "RAPIDSNARK_STATIC",
            if is_static_rapidsnark() { "1" } else { "0" },
        )
        .status()
        .unwrap_or_else(|e| panic!("Failed to run the rapidsnark download script: {e}"));

    if !status.success() {
        panic!(
            "Failed to fetch the rapidsnark prebuilt for {target}. Set {BASE_URL_ENV} to fetch \
             from a mirror, or {LIB_DIR_ENV} to link libraries you already have."
        );
    }

    lib_path
}

fn main() {
    for name in [
        "NOA_FRIDA_GUM_DEVKIT",
        "NOA_LSPLANT_SHIM",
        "NOA_LSPLANT_BLOB",
        "NOA_CXX_RUNTIME_DIR",
        "NOA_COMPILER_RUNTIME",
    ] {
        println!("cargo:rerun-if-env-changed={name}");
    }
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("android") {
        let placeholder =
            std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("empty-lsplant.so");
        std::fs::write(&placeholder, []).unwrap();
        println!("cargo:rustc-env=NOA_LSPLANT_BLOB={}", placeholder.display());
        return;
    }
    let gum = std::env::var("NOA_FRIDA_GUM_DEVKIT").expect("NOA_FRIDA_GUM_DEVKIT is required");
    let shim = std::env::var("NOA_LSPLANT_SHIM").expect("NOA_LSPLANT_SHIM is required");
    let lsplant = std::env::var("NOA_LSPLANT_BLOB").expect("NOA_LSPLANT_BLOB is required");
    let cxx_runtime =
        std::env::var("NOA_CXX_RUNTIME_DIR").expect("NOA_CXX_RUNTIME_DIR is required");
    let compiler_runtime =
        std::env::var("NOA_COMPILER_RUNTIME").expect("NOA_COMPILER_RUNTIME is required");
    let shim_directory = std::path::Path::new(&shim).parent().unwrap();
    println!("cargo:rerun-if-changed={shim}");
    println!("cargo:rerun-if-changed={lsplant}");
    println!("cargo:rerun-if-changed={compiler_runtime}");
    println!("cargo:rustc-env=NOA_LSPLANT_BLOB={lsplant}");
    println!(
        "cargo:rustc-link-search=native={}",
        shim_directory.display()
    );
    println!("cargo:rustc-link-search=native={gum}");
    println!("cargo:rustc-link-lib=static=noa_lsplant_shim");
    println!("cargo:rustc-link-lib=static=frida-gum");
    println!("cargo:rustc-link-arg={cxx_runtime}/libc++_static.a");
    println!("cargo:rustc-link-arg={cxx_runtime}/libc++abi.a");
    println!("cargo:rustc-link-arg={compiler_runtime}");
    println!("cargo:rustc-link-lib=log");
    println!("cargo:rustc-link-lib=dl");
    println!("cargo:rustc-link-lib=m");
}

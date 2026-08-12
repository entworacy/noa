fn main() {
    let gum = std::env::var("NOA_FRIDA_GUM_DEVKIT").expect("NOA_FRIDA_GUM_DEVKIT is required");
    let shim = std::env::var("NOA_LSPLANT_SHIM").expect("NOA_LSPLANT_SHIM is required");
    let lsplant = std::env::var("NOA_LSPLANT_BLOB").expect("NOA_LSPLANT_BLOB is required");
    let cxx = std::env::var("NOA_CXX_STATIC").expect("NOA_CXX_STATIC is required");
    let shim_directory = std::path::Path::new(&shim).parent().unwrap();
    println!("cargo:rustc-env=NOA_LSPLANT_BLOB={lsplant}");
    println!(
        "cargo:rustc-link-search=native={}",
        shim_directory.display()
    );
    println!("cargo:rustc-link-search=native={gum}");
    println!("cargo:rustc-link-lib=static=noa_lsplant_shim");
    println!("cargo:rustc-link-lib=static=frida-gum");
    println!("cargo:rustc-link-arg={cxx}");
    println!("cargo:rustc-link-lib=log");
    println!("cargo:rustc-link-lib=dl");
    println!("cargo:rustc-link-lib=m");
}

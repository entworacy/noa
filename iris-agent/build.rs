fn main() {
    let gum = std::env::var("NOA_FRIDA_GUM_DEVKIT")
        .expect("NOA_FRIDA_GUM_DEVKIT must point to a Frida Gum Android devkit");
    let shim = std::env::var("NOA_LSPLANT_SHIM")
        .expect("NOA_LSPLANT_SHIM must point to the LSPlant shim archive");
    let lsplant = std::env::var("NOA_LSPLANT_BLOB")
        .expect("NOA_LSPLANT_BLOB must point to the LSPlant Android shared library");
    let cxx = std::env::var("NOA_CXX_STATIC")
        .expect("NOA_CXX_STATIC must point to the Android static C++ runtime");
    let compiler_runtime = std::env::var("NOA_COMPILER_RUNTIME")
        .expect("NOA_COMPILER_RUNTIME must point to the Android compiler runtime");

    for (name, path) in [
        ("NOA_LSPLANT_SHIM", &shim),
        ("NOA_LSPLANT_BLOB", &lsplant),
        ("NOA_CXX_STATIC", &cxx),
        ("NOA_COMPILER_RUNTIME", &compiler_runtime),
    ] {
        assert!(
            std::path::Path::new(path).is_file(),
            "{name} not found: {path}"
        );
        println!("cargo:rerun-if-env-changed={name}");
    }
    let gum_archive = std::path::Path::new(&gum).join("libfrida-gum.a");
    assert!(
        gum_archive.is_file(),
        "Frida Gum archive not found: {}",
        gum_archive.display()
    );
    let shim_directory = std::path::Path::new(&shim)
        .parent()
        .expect("LSPlant shim path has no parent");

    println!("cargo:rustc-env=NOA_LSPLANT_BLOB={lsplant}");
    println!(
        "cargo:rustc-link-search=native={}",
        shim_directory.display()
    );
    println!("cargo:rustc-link-search=native={gum}");
    println!("cargo:rustc-link-lib=static=noa_lsplant_shim");
    println!("cargo:rustc-link-lib=static=frida-gum");
    println!("cargo:rustc-link-arg={cxx}");
    println!("cargo:rustc-link-arg={compiler_runtime}");
    println!("cargo:rustc-link-lib=log");
    println!("cargo:rustc-link-lib=dl");
    println!("cargo:rustc-link-lib=m");
    println!("cargo:rerun-if-changed=native/lsplant_shim.cpp");
    println!("cargo:rerun-if-changed=native/lsplant_api.hpp");
}

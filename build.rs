fn main() {
    publish_revision();
    if android_target() {
        link_frida_core();
        embed_kakao_agent();
        embed_iris_agent();
        for name in runtime_exports() {
            println!("cargo:rustc-link-arg-bins=-Wl,-u,{name}");
            println!("cargo:rustc-link-arg-bins=-Wl,--export-dynamic-symbol={name}");
        }
    }
}

fn embed_iris_agent() {
    println!("cargo:rerun-if-env-changed=NOA_IRIS_AGENT_BLOB");
    let path = std::env::var("NOA_IRIS_AGENT_BLOB")
        .expect("NOA_IRIS_AGENT_BLOB must point to the Android Iris agent shared library");
    assert!(
        std::path::Path::new(&path).is_file(),
        "Iris agent shared library not found: {path}"
    );
    println!("cargo:rustc-env=NOA_IRIS_AGENT_BLOB={path}");
}

fn embed_kakao_agent() {
    println!("cargo:rerun-if-env-changed=NOA_KAKAO_AGENT_BLOB");
    let path = std::env::var("NOA_KAKAO_AGENT_BLOB")
        .expect("NOA_KAKAO_AGENT_BLOB must point to the Android Kakao agent shared library");
    assert!(
        std::path::Path::new(&path).is_file(),
        "Kakao agent shared library not found: {path}"
    );
    println!("cargo:rustc-env=NOA_KAKAO_AGENT_BLOB={path}");
}

fn link_frida_core() {
    println!("cargo:rerun-if-env-changed=NOA_FRIDA_CORE_DEVKIT");
    let directory = std::env::var("NOA_FRIDA_CORE_DEVKIT")
        .expect("NOA_FRIDA_CORE_DEVKIT must point to a Frida Core Android devkit");
    let archive = std::path::Path::new(&directory).join("libfrida-core.a");
    assert!(
        archive.is_file(),
        "Frida Core archive not found: {}",
        archive.display()
    );
    println!("cargo:rustc-link-search=native={directory}");
    println!("cargo:rustc-link-lib=static=frida-core");
    println!("cargo:rustc-link-lib=log");
    println!("cargo:rustc-link-lib=dl");
    println!("cargo:rustc-link-lib=m");
    println!("cargo:rustc-link-arg-bins=-Wl,--export-dynamic");
}

fn publish_revision() {
    println!("cargo:rerun-if-env-changed=NOA_BUILD_REVISION");
    let revision = std::env::var("NOA_BUILD_REVISION")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "development".to_string());
    println!("cargo:rustc-env=NOA_BUILD_REVISION={revision}");
}

fn android_target() -> bool {
    std::env::var("TARGET")
        .map(|target| target.contains("android"))
        .unwrap_or(false)
}

fn runtime_exports() -> impl Iterator<Item = &'static str> {
    [
        "InitializeSignalChain",
        "EnsureFrontOfChain",
        "SetSpecialSignalHandlerFn",
        "GetSpecialSignalHandlerFn",
        "AddSpecialSignalHandlerFn",
        "RemoveSpecialSignalHandlerFn",
    ]
    .into_iter()
}

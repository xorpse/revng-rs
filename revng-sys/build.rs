use revng_build::Sdk;

fn main() {
    let sdk = Sdk::discover().unwrap_or_else(|error| panic!("{error}"));
    println!("cargo::metadata=sdk={}", sdk.prefix().display());
    println!("cargo::metadata=llvm={}", sdk.llvm().display());
    println!("cargo::metadata=llvm_major={}", sdk.llvm_major());
    println!(
        "cargo::rustc-env=REVNG_SDK_INCLUDE={}",
        sdk.prefix().join("include").display()
    );
    println!(
        "cargo::rustc-env=REVNG_SDK_LIB={}",
        sdk.prefix().join("lib").display()
    );
}

//! Requires every function this crate declares to exist in the library.
//!
//! A declaration that no longer matches an export links fine until the loader
//! resolves it, which for a `dlopen`-style consumer can be long after the
//! build. Renames are exactly what an upstream sync produces, so check the
//! whole set against the shipped symbol table instead.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use object::{File, Object};

fn declared_functions() -> BTreeSet<String> {
    include_str!("../src/lib.rs")
        .lines()
        .filter_map(|line| {
            let rest = line.trim().strip_prefix("pub fn ")?;
            let name = rest.split(['(', '<']).next()?;
            Some(name.to_owned())
        })
        .collect()
}

fn exported_symbols(library: &Path) -> BTreeSet<String> {
    let shown = library.display();
    let data = fs::read(library).unwrap_or_else(|error| panic!("could not read {shown}: {error}"));
    let parsed =
        File::parse(&*data).unwrap_or_else(|error| panic!("could not parse {shown}: {error}"));
    parsed
        .exports()
        .unwrap_or_else(|error| panic!("could not read the exports of {shown}: {error}"))
        .into_iter()
        .map(|export| {
            let name = String::from_utf8_lossy(export.name());
            // Mach-O prefixes every C symbol with an underscore; ELF does not.
            name.strip_prefix('_').unwrap_or(&name).to_owned()
        })
        .collect()
}

#[test]
fn every_declared_function_is_exported() {
    let library = PathBuf::from(env!("REVNG_SDK_LIB")).join(if cfg!(target_os = "macos") {
        "librevngPipelineC.dylib"
    } else {
        "librevngPipelineC.so"
    });
    assert!(library.exists(), "{} is absent", library.display());

    let declared = declared_functions();
    assert!(
        declared.len() > 50,
        "only found {} declarations; the parser has stopped working",
        declared.len(),
    );

    let exported = exported_symbols(&library);
    let missing = declared.difference(&exported).collect::<Vec<_>>();
    assert!(
        missing.is_empty(),
        "{} of {} declared functions are not exported by {}: {missing:?}",
        missing.len(),
        declared.len(),
        library.display(),
    );
}

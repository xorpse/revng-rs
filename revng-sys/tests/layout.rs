//! Checks every `#[repr(C)]` declaration against the header it mirrors.
//!
//! These bindings are written by hand, so nothing otherwise notices when a
//! field is added upstream, reordered, or widened: the declaration stays
//! plausible and the mismatch only shows up as a wrong pointer at run time.
//! This compiles a probe against the real installed header and requires the
//! two to agree on every size, alignment, offset and enumerator.
//!
//! One list drives both sides, so a type cannot be checked here and forgotten
//! in the probe.

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::mem::{MaybeUninit, align_of, offset_of, size_of};
use std::path::PathBuf;
use std::process::{Command, id};

use revng_sys::*;

struct Field {
    c_name: &'static str,
    offset: usize,
    size: usize,
}

struct Record {
    c_name: &'static str,
    size: usize,
    align: usize,
    fields: Vec<Field>,
}

struct Enumerator {
    c_name: &'static str,
    value: i64,
}

struct Enumeration {
    c_name: &'static str,
    size: usize,
    align: usize,
    variants: Vec<Enumerator>,
}

/// Recovers a field's width from a pointer to it, since `offset_of!` alone
/// cannot tell a `u32` from a `u64` that padding has put at the same offset.
fn pointee_size<T>(_: *const T) -> usize {
    size_of::<T>()
}

fn probe_type_line(c_name: &str) -> String {
    format!("  printf(\"T {c_name} %zu %zu\\n\", sizeof({c_name}), _Alignof({c_name}));\n")
}

/// `$field as "name"` covers the fields Rust has to rename, such as `type_`.
macro_rules! records {
    ($($ty:ident { $($field:ident $(as $c_field:literal)?),* $(,)? }),* $(,)?) => {
        fn rust_records() -> Vec<Record> {
            vec![$(Record {
                c_name: stringify!($ty),
                size: size_of::<$ty>(),
                align: align_of::<$ty>(),
                fields: vec![$(Field {
                    c_name: records!(@name $field $(, $c_field)?),
                    offset: offset_of!($ty, $field),
                    size: {
                        let uninit = MaybeUninit::<$ty>::uninit();
                        let base = uninit.as_ptr();
                        // SAFETY: `&raw const` forms a raw pointer to a place
                        // without reading it, so the memory stays untouched.
                        pointee_size(unsafe { &raw const (*base).$field })
                    },
                }),*],
            }),*]
        }

        fn probe_records() -> String {
            let mut out = String::new();
            $(
                out.push_str(&probe_type_line(stringify!($ty)));
                $(
                    out.push_str(&format!(
                        "  printf(\"F {0} {1} %zu %zu\\n\", offsetof({0}, {1}), \
                         sizeof((({0} *) 0)->{1}));\n",
                        stringify!($ty),
                        records!(@name $field $(, $c_field)?),
                    ));
                )*
            )*
            out
        }
    };
    (@name $field:ident) => { stringify!($field) };
    (@name $field:ident, $c_field:literal) => { $c_field };
}

macro_rules! enumerations {
    ($($ty:ident { $($variant:ident),* $(,)? }),* $(,)?) => {
        fn rust_enumerations() -> Vec<Enumeration> {
            vec![$(Enumeration {
                c_name: stringify!($ty),
                size: size_of::<$ty>(),
                align: align_of::<$ty>(),
                variants: vec![$(Enumerator {
                    c_name: stringify!($variant),
                    value: $ty::$variant as i64,
                }),*],
            }),*]
        }

        fn probe_enumerations() -> String {
            let mut out = String::new();
            $(
                out.push_str(&probe_type_line(stringify!($ty)));
                $(out.push_str(&format!(
                    "  printf(\"E {0} %lld\\n\", (long long) {0});\n",
                    stringify!($variant),
                ));)*
            )*
            out
        }
    };
}

records! {
    rp_mlir_module { ptr },
    rp_address_space_mapping {
        start, virtual_size, backing_size, readable, writeable, executable, name,
    },
    rp_address_space_callbacks {
        opaque, architecture, entry_point, mapping_count, mapping_at, read,
        extra_code_address_count, extra_code_address_at, release,
    },
    rp_file_address_space_mapping {
        start, virtual_size, backing_size, path, file_offset, readable, writeable,
        executable, name,
    },
    rp_lifter_callbacks { opaque, lift },
    rp_llvm_module_callbacks { opaque, transform },
    rp_mlir_module_callbacks { opaque, transform },
    rp_primitive_type { kind, size },
    rp_cabi_argument { name, type_ as "type" },
    rp_type { kind, is_const, primitive, size, definition_id, element_type },
    rp_typed_argument { name, comment, type_ as "type" },
    rp_named_typed_register { register_name, name, comment, type_ as "type" },
}

enumerations! {
    rp_primitive_kind {
        RP_PRIMITIVE_KIND_VOID,
        RP_PRIMITIVE_KIND_GENERIC,
        RP_PRIMITIVE_KIND_POINTER_OR_NUMBER,
        RP_PRIMITIVE_KIND_NUMBER,
        RP_PRIMITIVE_KIND_UNSIGNED,
        RP_PRIMITIVE_KIND_SIGNED,
        RP_PRIMITIVE_KIND_FLOAT,
    },
    rp_type_kind {
        RP_TYPE_KIND_PRIMITIVE,
        RP_TYPE_KIND_POINTER,
        RP_TYPE_KIND_ARRAY,
        RP_TYPE_KIND_DEFINED,
    },
}

fn probe_output() -> String {
    let include = PathBuf::from(env!("REVNG_SDK_INCLUDE"));
    let directory = env::temp_dir().join(format!("revng-sys-layout-{}", id()));
    fs::create_dir_all(&directory).expect("create scratch directory");

    let source = directory.join("probe.c");
    fs::write(
        &source,
        format!(
            "#include <revng/PipelineC/PipelineC.h>\n\
             #include <stddef.h>\n\
             #include <stdio.h>\n\
             \n\
             int main(void) {{\n{}{}  return 0;\n}}\n",
            probe_records(),
            probe_enumerations(),
        ),
    )
    .expect("write probe");

    let binary = directory.join("probe");
    let compiler = env::var("CC").unwrap_or_else(|_| "clang".to_owned());
    let compile = Command::new(&compiler)
        .arg("-std=c11")
        .arg("-Werror")
        .arg("-I")
        .arg(&include)
        .arg(&source)
        .arg("-o")
        .arg(&binary)
        .output()
        .unwrap_or_else(|error| panic!("could not run {compiler}: {error}"));
    assert!(
        compile.status.success(),
        "the probe did not compile against {}:\n{}",
        include.display(),
        String::from_utf8_lossy(&compile.stderr),
    );

    let run = Command::new(&binary).output().expect("run the probe");
    assert!(run.status.success(), "the probe exited with {}", run.status);
    let stdout = String::from_utf8(run.stdout).expect("probe output is UTF-8");
    let _ = fs::remove_dir_all(&directory);
    stdout
}

fn layout_problem(
    header: Option<(i64, i64)>,
    c_name: &str,
    size: usize,
    align: usize,
) -> Option<String> {
    let Some((found_size, found_align)) = header else {
        return Some(format!("{c_name}: absent from the header"));
    };
    if (found_size, found_align) == (size as i64, align as i64) {
        return None;
    }
    Some(format!(
        "{c_name}: Rust is {size} bytes aligned {align}, \
         the header {found_size} bytes aligned {found_align}"
    ))
}

#[test]
fn declarations_match_the_header() {
    let output = probe_output();

    let mut sizes = BTreeMap::new();
    let mut offsets = BTreeMap::new();
    let mut enum_values = BTreeMap::new();
    let number = |value: &str| {
        value
            .parse::<i64>()
            .unwrap_or_else(|_| panic!("probe printed a non-number: {value}"))
    };
    for line in output.lines() {
        match line.split_whitespace().collect::<Vec<_>>().as_slice() {
            ["T", name, size, align] => {
                sizes.insert((*name).to_owned(), (number(size), number(align)));
            }
            ["F", name, field, offset, size] => {
                offsets.insert(format!("{name}.{field}"), (number(offset), number(size)));
            }
            ["E", name, value] => {
                enum_values.insert((*name).to_owned(), number(value));
            }
            _ => panic!("unexpected probe output: {line}"),
        }
    }

    let mut problems = Vec::new();
    for record in rust_records() {
        problems.extend(layout_problem(
            sizes.get(record.c_name).copied(),
            record.c_name,
            record.size,
            record.align,
        ));
        for field in record.fields {
            let key = format!("{}.{}", record.c_name, field.c_name);
            let expected = (field.offset as i64, field.size as i64);
            match offsets.get(&key) {
                Some(&found) if found == expected => {}
                Some(&(offset, size)) => problems.push(format!(
                    "{key}: Rust puts {} bytes at {}, the header {size} bytes at {offset}",
                    field.size, field.offset,
                )),
                None => problems.push(format!("{key}: absent from the header")),
            }
        }
    }

    for enumeration in rust_enumerations() {
        problems.extend(layout_problem(
            sizes.get(enumeration.c_name).copied(),
            enumeration.c_name,
            enumeration.size,
            enumeration.align,
        ));
        for variant in enumeration.variants {
            match enum_values.get(variant.c_name) {
                Some(&found) if found == variant.value => {}
                Some(&found) => problems.push(format!(
                    "{}: Rust says {}, the header {found}",
                    variant.c_name, variant.value,
                )),
                None => problems.push(format!("{}: absent from the header", variant.c_name)),
            }
        }
    }

    assert!(
        problems.is_empty(),
        "the bindings have drifted from the header:\n  {}",
        problems.join("\n  "),
    );
}

/// Guards the lists above against a newly declared type slipping past them.
#[test]
fn every_repr_c_type_is_covered() {
    let source = include_str!("../src/lib.rs");
    let mut declared = BTreeSet::new();
    let mut lines = source.lines();
    while let Some(line) = lines.next() {
        if line.trim() != "#[repr(C)]" {
            continue;
        }
        // Derives and doc comments may sit between the attribute and the item.
        for following in lines.by_ref() {
            let trimmed = following.trim();
            if let Some(rest) = trimmed
                .strip_prefix("pub struct ")
                .or_else(|| trimmed.strip_prefix("pub enum "))
            {
                let name = rest.trim_end_matches([' ', '{']);
                // The opaque handles are generated from a macro and are
                // deliberately zero-sized, so they have no layout to check.
                if name != "$name" {
                    declared.insert(name.to_owned());
                }
                break;
            }
            if !trimmed.starts_with('#') && !trimmed.starts_with("///") && !trimmed.is_empty() {
                break;
            }
        }
    }

    let covered = rust_records()
        .into_iter()
        .map(|record| record.c_name)
        .chain(
            rust_enumerations()
                .into_iter()
                .map(|enumeration| enumeration.c_name),
        )
        .collect::<BTreeSet<_>>();
    let unchecked = declared
        .iter()
        .filter(|name| !covered.contains(name.as_str()))
        .collect::<Vec<_>>();
    assert!(
        unchecked.is_empty(),
        "these `#[repr(C)]` types are not checked against the header: {unchecked:?}",
    );
    let undeclared = covered
        .iter()
        .filter(|name| !declared.contains(**name))
        .collect::<Vec<_>>();
    assert!(
        undeclared.is_empty(),
        "the lists check types the crate does not declare: {undeclared:?}",
    );
}

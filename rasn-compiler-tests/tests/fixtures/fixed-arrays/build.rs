use rasn_compiler::prelude::{Compiler, RasnBackend};
use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=schema.asn1");
    let schema = fs::read_to_string("schema.asn1").unwrap();
    for (file, enabled) in [("arrays.rs", true), ("vectors.rs", false)] {
        let result = Compiler::<RasnBackend, _>::new()
            .with_backend(RasnBackend::default().with_fixed_size_arrays(enabled))
            .add_asn_literal(&schema)
            .compile_to_string()
            .unwrap();
        assert!(result.warnings.is_empty(), "{:?}", result.warnings);
        fs::write(
            PathBuf::from(env::var_os("OUT_DIR").unwrap()).join(file),
            result.generated,
        )
        .unwrap();
    }
}

use rasn_compiler::prelude::*;

const SCHEMA: &str = r#"
Example DEFINITIONS AUTOMATIC TAGS ::= BEGIN
    Permission ::= ENUMERATED { installation-read, write }
    Shape ::= CHOICE { unit-point INTEGER, circle BOOLEAN }
    default-permission Permission ::= installation-read
END
"#;

fn compile(backend: RasnBackend) -> String {
    let result = Compiler::<RasnBackend, _>::new()
        .with_backend(backend)
        .add_asn_literal(SCHEMA)
        .compile_to_string()
        .unwrap();
    assert!(result.warnings.is_empty(), "{:?}", result.warnings);
    result
        .generated
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn upper_camel_case_variants_keep_their_asn_names() {
    let generated = compile(RasnBackend::default().with_upper_camel_case_variants(true));
    for expected in [
        r#"#[rasn(identifier = "installation-read")] InstallationRead = 0"#,
        "Write = 1",
        r#"#[rasn(identifier = "unit-point")] UnitPoint(Integer)"#,
        "Circle(bool)",
        "Permission::InstallationRead",
    ] {
        assert!(
            generated.contains(expected),
            "missing {expected}: {generated}"
        );
    }
}

#[test]
fn variants_keep_asn_spelling_by_default() {
    let generated = compile(RasnBackend::default());
    for expected in [
        "installation_read = 0",
        "write = 1",
        "unit_point(Integer)",
        "circle(bool)",
    ] {
        assert!(
            generated.contains(expected),
            "missing {expected}: {generated}"
        );
    }
}

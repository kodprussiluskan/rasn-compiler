use rasn_compiler::prelude::*;

const SCHEMA: &str = r#"
Numbers DEFINITIONS AUTOMATIC TAGS ::= BEGIN
    Float32 ::= REAL (WITH COMPONENTS { mantissa (-16777215..16777215), base (2), exponent (-149..104) })
    Point ::= SEQUENCE { x Float32, y Float32 }
    Points ::= SET OF Point
    Raw ::= SEQUENCE {
        value REAL (WITH COMPONENTS { mantissa (-16777215..16777215), base (2), exponent (-149..104) })
    }
    half Float32 ::= 0.5
END
"#;

fn compile(hashable: bool) -> Result<CompileResult, rasn_compiler::prelude::CompilerError> {
    Compiler::<RasnBackend, _>::new()
        .with_backend(RasnBackend::default().with_hashable_reals(hashable))
        .add_asn_literal(SCHEMA)
        .compile_to_string()
}

/// The derive list of the type declared right after it.
fn derives_of(generated: &str, declaration: &str) -> String {
    let at = generated
        .find(declaration)
        .unwrap_or_else(|| panic!("no {declaration}: {generated}"));
    let derive = generated[..at].rfind("#[derive(").unwrap();
    generated[derive..at].split(')').next().unwrap().to_string()
}

#[test]
fn a_real_type_is_a_hashable_newtype() {
    let result = compile(true).unwrap();
    assert!(result.warnings.is_empty(), "{:?}", result.warnings);
    let generated = result.generated;

    let float = derives_of(&generated, "pub struct Float32(pub f32)");
    let collapsed = generated.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(collapsed.contains("#[repr(transparent)] pub struct Float32"), "{generated}");
    assert!(
        !float.contains("PartialEq") && !float.contains("Hash"),
        "{float}"
    );
    for implementation in [
        "impl core::cmp::PartialEq for Float32",
        "impl core::cmp::Eq for Float32",
        "impl core::hash::Hash for Float32",
    ] {
        assert!(
            generated.contains(implementation),
            "missing {implementation}: {generated}"
        );
    }

    let point = derives_of(&generated, "pub struct Point");
    assert!(point.contains("Eq") && point.contains("Hash"), "{point}");
    assert!(
        generated.contains("pub struct Points(pub SetOf<Point>)"),
        "{generated}"
    );

    // A REAL written inline stays a plain float, so its type still can't be Eq or Hash.
    let raw = derives_of(&generated, "pub struct Raw");
    assert!(!raw.contains("Hash"), "{raw}");

    assert!(generated.contains("Float32(0.5"), "{generated}");
}

#[test]
fn without_the_option_a_real_type_is_an_alias_and_cannot_be_in_a_set_of() {
    let result = compile(false).unwrap();
    assert!(
        result.generated.contains("pub type Float32 = f32;"),
        "{}",
        result.generated
    );
    assert!(
        !result.generated.contains("pub struct Points"),
        "{}",
        result.generated
    );
    assert!(
        result
            .warnings
            .iter()
            .any(|warning| warning.to_string().contains("cannot implement Eq and Hash")),
        "{:?}",
        result.warnings
    );
}

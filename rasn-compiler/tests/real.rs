use rasn_compiler::RasnCompiler;

#[test]
fn real_width_uses_the_oer_component_bounds() {
    let result = RasnCompiler::new()
        .add_asn_literal(
            "Numbers DEFINITIONS ::= BEGIN
                Wider ::= REAL (WITH COMPONENTS {
                    mantissa (-9999999..9999999), base (2), exponent (-126..127)
                })
                Beyond ::= REAL (WITH COMPONENTS {
                    mantissa (-999999999999999..999999999999999), base (2), exponent (-1022..1023)
                })
            END",
        )
        .compile_to_string()
        .unwrap();

    assert!(result.generated.contains("pub type Wider = f64;"));
    assert!(!result.generated.contains("pub type Beyond"));
    assert_eq!(result.warnings.len(), 1, "{:?}", result.warnings);
    assert!(result.warnings[0]
        .to_string()
        .contains("OER binary32 or binary64"));
}

#[test]
fn unconstrained_real_constants_are_skipped() {
    let result = RasnCompiler::new()
        .add_asn_literal(
            "Numbers DEFINITIONS ::= BEGIN
                Plain ::= REAL
                named Plain ::= 1.5
                direct REAL ::= 1.5
            END",
        )
        .compile_to_string()
        .unwrap();

    assert_eq!(result.warnings.len(), 2, "{:?}", result.warnings);
    assert!(
        result.warnings.iter().all(|warning| warning
            .to_string()
            .contains("OER binary32 or binary64")),
        "{:?}",
        result.warnings
    );
    assert!(!result.generated.contains("pub const"));
}

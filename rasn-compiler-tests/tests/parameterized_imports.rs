use rasn_compiler::prelude::{Compiler, RasnBackend, RasnConfig, TypescriptBackend};

const ORIGIN: &str = "Origin DEFINITIONS ::= BEGIN
    EXPORTS List{}, Flag;
    List{T} ::= SEQUENCE SIZE (2) OF T
    Flag ::= BOOLEAN
END";

fn rust(asn: &str, wildcard: bool) -> rasn_compiler::CompileResult {
    Compiler::<RasnBackend, _>::new_with_config(RasnConfig {
        default_wildcard_imports: wildcard,
        ..Default::default()
    })
    .add_asn_literal(asn)
    .compile_to_string()
    .unwrap()
}

#[test]
fn omits_template_only_import() {
    for symbol in ["List", "List{}", "List { }", "List { /* marker */ }"] {
        let asn = format!("{ORIGIN}
            Consumer DEFINITIONS ::= BEGIN
            IMPORTS {symbol} FROM Origin;
            Flags ::= List{{BOOLEAN}}
            END");
        for wildcard in [false, true] {
            let result = rust(&asn, wildcard);
            assert!(result.warnings.is_empty(), "{:?}", result.warnings);
            assert!(result.generated.contains("struct Flags"));
            assert!(!result.generated.contains("use super::origin"), "{}", result.generated);
        }
        let result = Compiler::<TypescriptBackend, _>::new()
            .add_asn_literal(&asn)
            .compile_to_string()
            .unwrap();
        assert!(result.warnings.is_empty(), "{:?}", result.warnings);
        assert!(result.generated.contains("Flags"));
        assert!(!result.generated.contains("import List"), "{}", result.generated);
    }
}

#[test]
fn keeps_mixed_and_external_imports() {
    let result = rust(&format!("{ORIGIN}
        Consumer DEFINITIONS ::= BEGIN
        IMPORTS List{{}}, Flag FROM Origin ExternalFlag FROM External;
        Flags ::= List{{BOOLEAN}}
        NamedFlag ::= Flag
        Outside ::= ExternalFlag
        END"), false);
    assert!(result.generated.contains("use super::origin::Flag;"), "{}", result.generated);
    assert!(result.generated.contains("use super::external::ExternalFlag;"), "{}", result.generated);
    assert!(!result.generated.contains("origin::List"), "{}", result.generated);
}

#[test]
fn uses_qualified_template_on_collision() {
    let result = rust(&format!("{ORIGIN}
        Right DEFINITIONS ::= BEGIN List{{T}} ::= SEQUENCE SIZE (3) OF T END
        Consumer DEFINITIONS ::= BEGIN
        IMPORTS List FROM Origin List FROM Right;
        Two ::= Origin.List{{BOOLEAN}}
        Three ::= Right.List{{BOOLEAN}}
        Ambiguous ::= List{{BOOLEAN}}
        END"), false);
    assert!(result.generated.contains(r#"size("2")"#), "{}", result.generated);
    assert!(result.generated.contains(r#"size("3")"#), "{}", result.generated);
    assert!(result.warnings.iter().any(|error| error.to_string().contains("ambiguous")), "{:?}", result.warnings);
    assert!(!result.generated.contains("struct Ambiguous"));
}

#[test]
fn resolves_template_reexport_to_owner() {
    for exports in ["EXPORTS List {};", "EXPORTS ALL;", ""] {
        let result = rust(&format!("
            ZConsumer DEFINITIONS ::= BEGIN
            IMPORTS List FROM Middle;
            Flags ::= List{{BOOLEAN}}
            END
            Middle DEFINITIONS ::= BEGIN
            {exports}
            IMPORTS List FROM Origin;
            END
            {ORIGIN}"), true);
        assert!(result.warnings.is_empty(), "{:?}", result.warnings);
        assert!(result.generated.contains(r#"size("2")"#), "{}", result.generated);
        assert!(!result.generated.contains("use super::middle"), "{}", result.generated);
    }
}

#[test]
fn diagnoses_invalid_template_paths() {
    let hidden = ORIGIN.replace("EXPORTS List{}, Flag;", "EXPORTS;");
    let cases = [
        (format!("{hidden} Consumer DEFINITIONS ::= BEGIN IMPORTS List FROM Origin; Bad ::= List{{BOOLEAN}} Good ::= BOOLEAN END"), "export"),
        (format!("{ORIGIN} Consumer DEFINITIONS ::= BEGIN Bad ::= Origin.List{{BOOLEAN}} Good ::= BOOLEAN END"), "import"),
        (format!("{ORIGIN} Other DEFINITIONS ::= BEGIN List{{T}} ::= T END Middle DEFINITIONS ::= BEGIN IMPORTS List FROM Origin List FROM Other; END Consumer DEFINITIONS ::= BEGIN IMPORTS List FROM Middle; Bad ::= List{{BOOLEAN}} Good ::= BOOLEAN END"), "ambiguous"),
        ("Middle DEFINITIONS ::= BEGIN IMPORTS List FROM Other; END Other DEFINITIONS ::= BEGIN IMPORTS List FROM Middle; END Consumer DEFINITIONS ::= BEGIN IMPORTS List FROM Middle; Bad ::= List{BOOLEAN} Good ::= BOOLEAN END".into(), "cycle"),
    ];
    for (asn, expected) in cases {
        let result = rust(&asn, false);
        assert!(result.warnings.iter().any(|warning| warning.to_string().contains(expected)), "{expected}: {:?}", result.warnings);
        assert!(result.generated.contains("struct Good"));
        assert!(!result.generated.contains("struct Bad"));
    }
}

#[test]
fn generated_template_use_compiles() {
    rasn_compiler_derive::asn1!(r#"
        Origin DEFINITIONS ::= BEGIN
            EXPORTS List{};
            List{T} ::= SEQUENCE SIZE (2) OF T
        END
        Consumer DEFINITIONS ::= BEGIN
            IMPORTS List FROM Origin;
            Flags ::= List{BOOLEAN}
        END
    "#);
    let bytes = [0x30, 0x06, 0x01, 0x01, 0xff, 0x01, 0x01, 0x00];
    let flags: consumer::Flags = rasn::der::decode(&bytes).unwrap();
    assert_eq!(rasn::der::encode(&flags).unwrap(), bytes);
}

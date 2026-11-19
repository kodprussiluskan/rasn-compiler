use rasn_compiler::{RasnCompiler, TsCompiler};

const SOURCE: &str = "
    Base DEFINITIONS ::= BEGIN
        Address ::= RELATIVE-OID
        root Address ::= { 8571 }
    END

    App DEFINITIONS AUTOMATIC TAGS ::= BEGIN
        IMPORTS Address, root FROM Base;
        part INTEGER ::= 3
        prefix RELATIVE-OID ::= { root part }
        address Address ::= { prefix 2 }
        Record ::= SEQUENCE { address Address, oid RELATIVE-OID, absolute OBJECT IDENTIFIER }
    END
";

#[test]
fn relative_oid_keeps_its_type_and_expands_imported_prefixes() {
    let result = RasnCompiler::new()
        .add_asn_literal(SOURCE)
        .compile_to_string()
        .unwrap();

    assert!(result.warnings.is_empty(), "{:?}\n{}", result.warnings, result.generated);
    let rust = result.generated.split_whitespace().collect::<String>();
    assert!(rust.contains("pubstructAddress(pubRelativeOid);"));
    assert!(rust.contains("puboid:RelativeOid,"));
    assert!(rust.contains("pubabsolute:ObjectIdentifier,"));
    assert!(rust.contains("[8571u32,3u32,2u32"));
}

#[test]
fn relative_oid_uses_jer_dot_decimal() {
    let result = TsCompiler::new()
        .add_asn_literal(SOURCE)
        .compile_to_string()
        .unwrap();

    assert!(result.warnings.is_empty(), "{:?}\n{}", result.warnings, result.generated);
    assert!(result.generated.contains("export type Address = string;"));
    assert!(result.generated.contains(r#""8571.3.2""#));
}

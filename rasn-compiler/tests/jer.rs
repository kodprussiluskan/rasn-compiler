use rasn_compiler::prelude::*;

#[test]
fn generates_rust_attributes_for_the_six_instructions() {
    let schema = r#"
Example DEFINITIONS JER INSTRUCTIONS AUTOMATIC TAGS ::= BEGIN
    Payload ::= [BASE64] OCTET STRING
    Plain ::= [NOT BASE64] Payload
    Color ::= [TEXT ALL AS UPPERCASED, blue AS "bleu"] ENUMERATED { red, blue }
    Vehicle ::= [UNWRAPPED] CHOICE { count INTEGER, color Color }
    List ::= [OBJECT] SET OF SEQUENCE { key UTF8String, value INTEGER }
    Record ::= SEQUENCE {
        payload [NAME AS "base 64"] Payload,
        plain [NOT BASE64] Payload
    }
    ENCODING-CONTROL JER
    [ARRAY] SEQUENCE
END
"#;
    let result = Compiler::<RasnBackend, _>::new()
        .add_asn_literal(schema)
        .compile_to_string()
        .unwrap();
    assert!(result.warnings.is_empty(), "{:?}", result.warnings);
    let bindings = result.generated;
    for instruction in [
        "jer(base64",
        "jer(default",
        "jer(unwrapped",
        "jer(object",
        "jer(array",
        "\"base 64\"",
        "\"RED\"",
        "\"bleu\"",
    ] {
        assert!(bindings.contains(instruction), "missing {instruction}: {bindings}");
    }
}

#[test]
fn reports_invalid_final_rules() {
    for declaration in [
        "Broken ::= [JER:BASE64] INTEGER",
        "Broken ::= [JER:OBJECT] SET OF INTEGER",
        "Broken ::= [JER:UNWRAPPED] CHOICE { a INTEGER, b INTEGER }",
        "Broken ::= [JER:ARRAY] SEQUENCE { a NULL OPTIONAL }",
        r#"Broken ::= SEQUENCE { a INTEGER, b [JER:NAME AS "a"] BOOLEAN }"#,
        r#"Broken ::= [JER:TEXT b AS "a"] ENUMERATED { a, b }"#,
    ] {
        let schema = format!("Example DEFINITIONS ::= BEGIN {declaration} END");
        assert!(
            Compiler::<RasnBackend, _>::new()
                .add_asn_literal(schema)
                .compile_to_string()
                .is_err(),
            "{declaration}"
        );
    }
}

#[test]
fn import_control_and_local_prefixes_have_distinct_scopes() {
    let schema = r#"
Source DEFINITIONS ::= BEGIN
    Payload ::= [JER:BASE64] OCTET STRING
END

Receiver DEFINITIONS JER INSTRUCTIONS ::= BEGIN
    IMPORTS Payload FROM Source;
    Plain ::= Payload
    Restore ::= [BASE64] [APPLICATION 0] IMPLICIT Payload
    ENCODING-CONTROL JER
    [NOT BASE64] ALL IMPORTS FROM Source
END
"#;
    let result = Compiler::<RasnBackend, _>::new()
        .add_asn_literal(schema)
        .compile_to_string()
        .unwrap();
    assert!(result.warnings.is_empty(), "{:?}", result.warnings);
    let bindings = result.generated;
    assert_eq!(bindings.matches("jer(base64)").count(), 2);
    assert_eq!(bindings.matches("jer(default)").count(), 1);
}

#[test]
fn typescript_uses_the_instruction_specific_wire_shapes() {
    let schema = r#"
Example DEFINITIONS JER INSTRUCTIONS ::= BEGIN
    Label ::= [TEXT ALL AS UPPERCASED] ENUMERATED { red, blue }
    RawLabel ::= [NOT TEXT] Label
    List ::= [OBJECT] SET OF SEQUENCE { key UTF8String, value INTEGER }
    Tuple ::= [ARRAY] SEQUENCE { number INTEGER, item Label OPTIONAL }
    Union ::= [UNWRAPPED] CHOICE { number INTEGER, label Label }
    Map ::= SEQUENCE { number [NAME AS "new name"] INTEGER }
END
"#;
    let result = Compiler::<TypescriptBackend, _>::new()
        .add_asn_literal(schema)
        .compile_to_string()
        .unwrap();
    assert!(result.warnings.is_empty(), "{:?}", result.warnings);
    for expected in [
        r#"type Label = "RED" | "BLUE""#,
        r#"type RawLabel = "red" | "blue""#,
        r#"type List = { [key: string]: number }"#,
        r#"type Tuple = [number, (Label | null)?]"#,
        r#"type Union = number | Label"#,
        r#"type Map = { "new name": number }"#,
    ] {
        assert!(
            result.generated.contains(expected),
            "{}\n{}",
            expected,
            result.generated
        );
    }
}

#[test]
fn resolves_names_and_text_from_imported_values() {
    let schema = r#"
Source DEFINITIONS ::= BEGIN
    label UTF8String ::= "wire name"
END

Receiver DEFINITIONS JER INSTRUCTIONS AUTOMATIC TAGS ::= BEGIN
    IMPORTS label FROM Source;
    Record ::= SEQUENCE { item [NAME AS label] INTEGER }
    ChoiceText ::= [TEXT local AS label] ENUMERATED { local, other }
END
"#;
    let result = Compiler::<RasnBackend, _>::new()
        .add_asn_literal(schema)
        .compile_to_string()
        .unwrap();
    assert!(result.warnings.is_empty(), "{:?}", result.warnings);
    assert!(result.generated.contains(r#"name("item", "wire name")"#));
    assert!(result.generated.contains(r#"text("local", "wire name")"#));
}

#[test]
fn typescript_diagnoses_instruction_sensitive_value_constants() {
    let schema = r#"
Example DEFINITIONS JER INSTRUCTIONS ::= BEGIN
    Payload ::= [BASE64] OCTET STRING
    payload Payload ::= 'FF'H
END
"#;
    let result = Compiler::<TypescriptBackend, _>::new()
        .add_asn_literal(schema)
        .compile_to_string()
        .unwrap();
    assert!(
        result
            .warnings
            .iter()
            .any(|warning| warning.to_string().contains("value constants"))
    );
    assert!(!result.generated.contains("const payload"));
}

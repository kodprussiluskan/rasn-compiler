use rasn_compiler::prelude::{CompileResult, Compiler, RasnBackend};

fn compile(asn: &str) -> CompileResult {
    Compiler::<RasnBackend, _>::new()
        .add_asn_literal(asn)
        .compile_to_string()
        .unwrap()
}

#[test]
fn uses_caller_actuals_and_template_references() {
    let templates = "Templates DEFINITIONS ::= BEGIN
        EXPORTS Fixed{};
        count INTEGER ::= 7
        Flag ::= INTEGER
        Fixed{INTEGER:n,T} ::= SEQUENCE {
            local Flag DEFAULT count,
            items SEQUENCE SIZE(n) OF T
        }
        END";
    let consumer = "Consumer DEFINITIONS ::= BEGIN
        IMPORTS Fixed FROM Templates;
        count INTEGER ::= source
        source INTEGER ::= 3
        Flag ::= BOOLEAN
        Flags ::= Fixed{count,Flag}
        END";
    for asn in [
        format!("{templates} {consumer}"),
        format!("{consumer} {templates}"),
    ] {
        let result = compile(&asn);
        assert!(result.warnings.is_empty(), "{:?}", result.warnings);
        assert!(
            result
                .generated
                .contains("pub local: super::templates::Flag"),
            "{}",
            result.generated
        );
        assert!(
            result.generated.contains("pub items: SequenceOf<Flag>"),
            "{}",
            result.generated
        );
        assert!(
            result.generated.contains("size(\"3\")"),
            "{}",
            result.generated
        );
    }
}

#[test]
fn forwards_actuals_across_templates() {
    let result = compile(
        "Templates DEFINITIONS ::= BEGIN
        Inner{INTEGER:n,T} ::= SEQUENCE SIZE (n) OF T
        Outer{INTEGER:n,T} ::= Inner{n,T}
        One ::= Outer{1, BOOLEAN}
        Three ::= Outer{3, BOOLEAN}
        END",
    );
    assert!(result.warnings.is_empty(), "{:?}", result.warnings);
    for size in [1, 3] {
        assert!(
            result.generated.contains(&format!("size(\"{size}\")")),
            "{}",
            result.generated
        );
    }
}

#[test]
fn keeps_use_site_constraint() {
    let result = compile(
        "Templates DEFINITIONS ::= BEGIN
        Range{T} ::= SEQUENCE SIZE (1..4) OF T
        Three ::= Range{BOOLEAN} (SIZE (3))
        END",
    );
    assert!(result.warnings.is_empty(), "{:?}", result.warnings);
    assert!(
        result.generated.contains("size(\"3\")"),
        "{}",
        result.generated
    );
}

#[test]
fn validates_arity_kind_and_governor() {
    for (actuals, expected) in [
        ("1", "expected 2"),
        ("1, BOOLEAN, NULL", "expected 2"),
        ("BOOLEAN, BOOLEAN", "parameter n"),
        ("4, BOOLEAN", "governor"),
    ] {
        let result = compile(&format!(
            "Templates DEFINITIONS ::= BEGIN
            Fixed{{INTEGER (0..3):n,T}} ::= SEQUENCE SIZE (n) OF T
            Bad ::= Fixed{{{actuals}}}
            Good ::= BOOLEAN
            END"
        ));
        assert!(
            result
                .warnings
                .iter()
                .any(|error| error.to_string().contains(expected)),
            "{:?}",
            result.warnings
        );
        assert!(
            !result.generated.contains("struct Bad"),
            "{}",
            result.generated
        );
        assert!(result.generated.contains("struct Good"));
    }
}

#[test]
fn reports_cycles_and_keeps_independent_output() {
    for body in [
        "Loop{T} ::= SEQUENCE OF Loop{INTEGER}",
        "Loop{T} ::= SEQUENCE OF Other{T} Other{T} ::= SEQUENCE OF Loop{T}",
    ] {
        let result = compile(&format!(
            "Templates DEFINITIONS ::= BEGIN
            {body}
            Bad ::= Loop{{BOOLEAN}}
            Dependent ::= SEQUENCE {{ value Bad }}
            Good ::= BOOLEAN
            END"
        ));
        assert!(
            result
                .warnings
                .iter()
                .any(|error| error.to_string().contains("cycle")),
            "{:?}",
            result.warnings
        );
        assert!(
            !result.generated.contains("struct Bad"),
            "{}",
            result.generated
        );
        assert!(
            !result.generated.contains("struct Dependent"),
            "{}",
            result.generated
        );
        assert!(result.generated.contains("struct Good"));
    }
}

#[test]
fn preserves_tags_across_environments() {
    rasn_compiler_derive::asn1!(
        "Templates DEFINITIONS EXPLICIT TAGS ::= BEGIN
        EXPORTS Tagged{};
        Tagged{T} ::= [5] EXPLICIT T
        END
        Consumer DEFINITIONS IMPLICIT TAGS ::= BEGIN
        IMPORTS Tagged FROM Templates;
        Item ::= [6] EXPLICIT Tagged{[2] BOOLEAN}
        END"
    );
    let expected = [0xa6, 0x05, 0xa5, 0x03, 0x82, 0x01, 0xff];
    let instance = rasn::der::decode::<consumer::Item>(&expected).unwrap();
    assert_eq!(rasn::der::encode(&instance).unwrap(), expected);

    rasn_compiler_derive::asn1!(
        "AutomaticTemplate DEFINITIONS AUTOMATIC TAGS ::= BEGIN
        EXPORTS Pair{};
        Pair{T} ::= SEQUENCE { first INTEGER, second T }
        END
        ExplicitCaller DEFINITIONS EXPLICIT TAGS ::= BEGIN
        IMPORTS Pair FROM AutomaticTemplate;
        Item ::= Pair{BOOLEAN}
        END"
    );
    let expected = [0x30, 0x08, 0x80, 0x01, 0x01, 0xa1, 0x03, 0x01, 0x01, 0xff];
    let instance = rasn::der::decode::<explicit_caller::Item>(&expected).unwrap();
    assert_eq!(rasn::der::encode(&instance).unwrap(), expected);
}

#[test]
fn strict_compilation_rejects_specialization_errors() {
    let error = Compiler::<RasnBackend, _>::new()
        .add_asn_literal(
            "Templates DEFINITIONS ::= BEGIN
            Fixed{INTEGER (0..3):n} ::= SEQUENCE SIZE (n) OF BOOLEAN
            Bad ::= Fixed{4}
            Good ::= BOOLEAN
            END",
        )
        .compile_to_string_strict()
        .unwrap_err();
    assert!(error.to_string().contains("Bad"), "{error}");
    assert!(error.to_string().contains("governor"), "{error}");
}

#[test]
fn allows_finite_template_nesting_at_the_use_site() {
    let result = compile(
        "Templates DEFINITIONS ::= BEGIN
        List{T} ::= SEQUENCE SIZE (2) OF T
        TwoByTwo ::= List{List{BOOLEAN}}
        END",
    );
    assert!(result.warnings.is_empty(), "{:?}", result.warnings);
    assert!(result.generated.contains("struct TwoByTwo"));
}

#[test]
fn preserves_explicit_template_environment_in_automatic_caller() {
    rasn_compiler_derive::asn1!(
        "ExplicitTemplate DEFINITIONS EXPLICIT TAGS ::= BEGIN
        EXPORTS Pair{};
        Pair{T} ::= SEQUENCE { first INTEGER, second T }
        END
        AutomaticCaller DEFINITIONS AUTOMATIC TAGS ::= BEGIN
        IMPORTS Pair FROM ExplicitTemplate;
        Item ::= Pair{BOOLEAN}
        END"
    );
    let expected = [0x30, 0x06, 0x02, 0x01, 0x01, 0x01, 0x01, 0xff];
    let instance = rasn::der::decode::<automatic_caller::Item>(&expected).unwrap();
    assert_eq!(rasn::der::encode(&instance).unwrap(), expected);
}

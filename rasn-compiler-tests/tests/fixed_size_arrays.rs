use rasn_compiler::prelude::{Backend, Compiler, RasnBackend, RasnConfig};

fn compile(body: &str, enabled: bool) -> rasn_compiler::CompileResult {
    Compiler::<RasnBackend, _>::new()
        .with_backend(RasnBackend::default().with_fixed_size_arrays(enabled))
        .add_asn_literal(format!(
            "Test DEFINITIONS AUTOMATIC TAGS ::= BEGIN {body} END"
        ))
        .compile_to_string()
        .unwrap()
}

#[test]
fn requires_opt_in_for_public_arrays() {
    let body = "Flags ::= SEQUENCE SIZE (3) OF BOOLEAN";
    let default = Compiler::<RasnBackend, _>::new_with_config(RasnConfig {
        opaque_open_types: false,
        ..Default::default()
    })
    .add_asn_literal(format!(
        "Test DEFINITIONS AUTOMATIC TAGS ::= BEGIN {body} END"
    ))
    .compile_to_string()
    .unwrap();
    assert_eq!(default.generated, compile(body, false).generated);
    assert!(default.generated.contains("SequenceOf<AnonymousFlags>"));
    assert!(compile(body, true)
        .generated
        .contains("[AnonymousFlags; 3]"));
}

#[test]
fn rejects_invalid_collection_values() {
    for enabled in [false, true] {
        for value in [
            "{TRUE, FALSE}",
            "{TRUE, FALSE, TRUE, FALSE}",
            "{TRUE, 1, FALSE}",
        ] {
            let result = compile(
                &format!("Flags ::= SEQUENCE SIZE (3) OF BOOLEAN bad Flags ::= {value}"),
                enabled,
            );
            assert!(!result.warnings.is_empty(), "{value}: {}", result.generated);
            assert!(
                !result.generated.contains("static BAD"),
                "{}",
                result.generated
            );
        }
        let result = compile(
            "Record ::= SEQUENCE { flags SEQUENCE SIZE (3) OF BOOLEAN DEFAULT {TRUE, FALSE} }",
            enabled,
        );
        assert!(result
            .warnings
            .iter()
            .any(|warning| warning.to_string().contains("length")));
        assert!(
            !result.generated.contains("struct Record"),
            "{}",
            result.generated
        );
    }
}

#[test]
fn standalone_generation_keeps_heap_indirection() {
    let source = "Test DEFINITIONS ::= BEGIN
        Tree ::= SEQUENCE { children SEQUENCE SIZE (1) OF Tree OPTIONAL }
        END";
    let generated = Compiler::<RasnBackend, _>::new()
        .with_backend(RasnBackend::default().with_fixed_size_arrays(true))
        .add_asn_literal(source)
        .compile_to_string()
        .unwrap();
    assert!(generated.warnings.is_empty());
    assert!(generated.generated.contains("SequenceOf<Tree>"));
    // A direct backend call has no batch graph available.
    let backend = RasnBackend::default().with_fixed_size_arrays(true);
    use rasn_compiler::prelude::ir::{
        ASN1Type, ASN1Value, Constraint, ElementOrSetOperation, ElementSetSpecs, SubtypeElements,
        ToplevelTypeDefinition,
    };
    let declaration = ToplevelTypeDefinition {
        comments: String::new(),
        name: "Flags".into(),
        parameterization: None,
        tag: None,
        module_header: None,
        jer: Default::default(),
        ty: ASN1Type::SequenceOf(rasn_compiler::prelude::ir::SequenceOrSetOf {
            constraints: vec![Constraint::Subtype(ElementSetSpecs {
                set: ElementOrSetOperation::Element(SubtypeElements::SizeConstraint(Box::new(
                    ElementOrSetOperation::Element(SubtypeElements::SingleValue {
                        value: ASN1Value::Integer(3),
                        extensible: false,
                    }),
                ))),
                extensible: false,
            })],
            element_type: Box::new(ASN1Type::Boolean(None.into())),
            element_tag: None,
            is_recursive: false,
        }),
    };
    let generated = backend.generate(rasn_compiler::prelude::ToplevelDefinition::Type(
        declaration,
    ));
    assert!(generated.is_ok());
    assert!(generated.unwrap().contains("SequenceOf"));
}

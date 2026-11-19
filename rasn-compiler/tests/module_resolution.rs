use rasn_compiler::RasnCompiler;

#[test]
fn same_named_types_resolve_in_their_own_modules() {
    let a = "A DEFINITIONS ::= BEGIN
        Shared ::= INTEGER (0..255)
        a Shared ::= 1
        END";
    let b = "B DEFINITIONS ::= BEGIN
        Shared ::= BOOLEAN
        b Shared ::= TRUE
        END";
    let c = "C DEFINITIONS ::= BEGIN
        IMPORTS Shared FROM A;
        c Shared ::= 2
        END";

    let compile = |first, second| {
        let result = RasnCompiler::new()
            .add_asn_literal(first)
            .add_asn_literal(second)
            .add_asn_literal(c)
            .compile_to_string()
            .unwrap();
        assert!(result.warnings.is_empty(), "{:?}", result.warnings);
        result.generated.split_whitespace().collect::<String>()
    };

    let generated = compile(a, b);
    assert_eq!(generated, compile(b, a));
    assert!(generated.contains("pubstructShared(pubu8);"));
    assert!(generated.contains("pubstructShared(pubbool);"));
    assert!(generated.contains("pubconstA:Shared=Shared(1);"));
    assert!(generated.contains("pubconstB:Shared=Shared(true);"));
    assert!(
        generated.contains("usesuper::a::{Shared};") || generated.contains("usesuper::a::Shared;")
    );
    assert!(generated.contains("pubconstC:Shared=Shared(2);"));
}

#[test]
fn typescript_keeps_qualified_imports_distinct() {
    let result = rasn_compiler::TsCompiler::new()
        .add_asn_literal(
            "A DEFINITIONS ::= BEGIN Shared ::= INTEGER END
             B DEFINITIONS ::= BEGIN Shared ::= BOOLEAN END
             C DEFINITIONS ::= BEGIN
                 IMPORTS Shared FROM A Shared FROM B;
                 Number ::= A.Shared
                 Flag ::= B.Shared
             END",
        )
        .compile_to_string()
        .unwrap();

    assert!(result.warnings.is_empty(), "{:?}", result.warnings);
    assert!(result.generated.contains("export type Shared = number;"));
    assert!(result.generated.contains("export type Shared = boolean;"));
    assert!(result.generated.contains("export type Number = A.Shared;"));
    assert!(result.generated.contains("export type Flag = B.Shared;"));
    assert!(!result.generated.contains("import Shared ="));
}

#[test]
fn warns_about_ambiguous_unqualified_types() {
    let result = RasnCompiler::new()
        .add_asn_literal(
            "A DEFINITIONS ::= BEGIN Shared ::= INTEGER END
             B DEFINITIONS ::= BEGIN Shared ::= BOOLEAN END
             C DEFINITIONS ::= BEGIN
                 IMPORTS Shared FROM A Shared FROM B;
                 Alias ::= Shared
             END",
        )
        .compile_to_string()
        .unwrap();

    assert!(result.warnings.iter().any(|warning| warning
        .to_string()
        .contains("Ambiguous type reference 'Shared'")));
}

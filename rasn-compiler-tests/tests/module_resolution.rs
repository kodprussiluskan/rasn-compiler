use rasn_compiler_derive::asn1;

asn1!(
    r#"
    A DEFINITIONS ::= BEGIN
        Shared ::= INTEGER (0..255)
    END

    B DEFINITIONS ::= BEGIN
        Shared ::= BOOLEAN
    END

    C DEFINITIONS AUTOMATIC TAGS ::= BEGIN
        IMPORTS Shared FROM A Shared FROM B;
        Pair ::= SEQUENCE { number A.Shared, flag B.Shared }
        number A.Shared ::= 42
        flag B.Shared ::= TRUE
    END

    D DEFINITIONS AUTOMATIC TAGS ::= BEGIN
        IMPORTS Shared FROM A;
        Shared ::= BOOLEAN
        Pair ::= SEQUENCE { number A.Shared, flag Shared }
    END
    "#
);

#[test]
fn qualified_imports_bind_to_distinct_rust_types() {
    assert_eq!(c::NUMBER, a::Shared(42));
    assert_eq!(c::FLAG, b::Shared(true));
    let pair = c::Pair::new(c::NUMBER, c::FLAG);
    assert_eq!(pair.number, a::Shared(42));
    assert_eq!(pair.flag, b::Shared(true));
    let pair = d::Pair::new(a::Shared(42), d::Shared(true));
    assert_eq!(pair.number, a::Shared(42));
    assert_eq!(pair.flag, d::Shared(true));
}

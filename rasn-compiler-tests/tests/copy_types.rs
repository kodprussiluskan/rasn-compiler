use rasn_compiler::{prelude::RasnConfig, RasnCompiler};
use rasn_compiler_derive::asn1;

mod caller_types {
    pub type ExternalBytes = rasn::types::OctetString;
}

asn1!(
    r#"
    Aaa-User DEFINITIONS AUTOMATIC TAGS ::= BEGIN
        IMPORTS CountAlias FROM Middle;
        Packet ::= SEQUENCE {
            count CountAlias OPTIONAL,
            selected Selection,
            flag BOOLEAN DEFAULT TRUE,
            nested SEQUENCE { value INTEGER (-10..10) },
            octets FixedBytes,
            bits FixedBits,
            timestamp Timestamp
        }
        Selection ::= CHOICE { number CountAlias, flag BOOLEAN }
        Pair ::= SET { number CountAlias, flag BOOLEAN }
        FixedBytes ::= OCTET STRING (SIZE(4))
        FixedBits ::= BIT STRING (SIZE(8))
        Timestamp ::= UTCTime
        Empty ::= SEQUENCE {}
        NumberList ::= SEQUENCE OF CountAlias
        ContainsList ::= SEQUENCE { values NumberList }
        ContainsInlineOctets ::= SEQUENCE { bytes OCTET STRING (SIZE(4)) }
        ContainsUnbounded ::= SEQUENCE { value INTEGER }
        ContainsExtensible ::= SEQUENCE { value INTEGER (0..255, ...) }
        Recursive ::= SEQUENCE { next Recursive OPTIONAL }
    END

    Shadow DEFINITIONS AUTOMATIC TAGS ::= BEGIN
        UtcTime ::= OCTET STRING
        TimeWithShadow ::= SEQUENCE { time UTCTime }
    END

    External-User DEFINITIONS AUTOMATIC TAGS ::= BEGIN
        IMPORTS ExternalBytes FROM Caller-Types;
        ContainsExternal ::= SEQUENCE { bytes ExternalBytes }
    END

    Middle DEFINITIONS AUTOMATIC TAGS ::= BEGIN
        IMPORTS Count FROM Zzz-Base;
        CountAlias ::= Count
    END

    Zzz-Base DEFINITIONS AUTOMATIC TAGS ::= BEGIN
        Count ::= INTEGER (0..255)
    END
    "#
);

#[test]
fn derives_copy_from_generated_field_storage() {
    fn assert_copy<T: Copy>() {}
    assert_copy::<aaa_user::Packet>();
    assert_copy::<aaa_user::PacketNested>();
    assert_copy::<aaa_user::Selection>();
    assert_copy::<aaa_user::Pair>();
    assert_copy::<aaa_user::Empty>();
    assert_copy::<aaa_user::FixedBytes>();
    assert_copy::<aaa_user::FixedBits>();
    assert_copy::<aaa_user::Timestamp>();
    assert_copy::<middle::CountAlias>();

    // Inference is ambiguous only if the tested type implements Copy.
    trait AmbiguousIfCopy<T> {
        fn check() {}
    }
    impl<T> AmbiguousIfCopy<()> for T {}
    impl<T: Copy> AmbiguousIfCopy<bool> for T {}

    let _ = <aaa_user::ContainsList as AmbiguousIfCopy<_>>::check;
    let _ = <aaa_user::ContainsInlineOctets as AmbiguousIfCopy<_>>::check;
    let _ = <aaa_user::ContainsUnbounded as AmbiguousIfCopy<_>>::check;
    let _ = <aaa_user::ContainsExtensible as AmbiguousIfCopy<_>>::check;
    let _ = <aaa_user::Recursive as AmbiguousIfCopy<_>>::check;
    let _ = <shadow::TimeWithShadow as AmbiguousIfCopy<_>>::check;
    let _ = <external_user::ContainsExternal as AmbiguousIfCopy<_>>::check;

    let pair = aaa_user::Pair {
        number: middle::CountAlias(zzz_base::Count(7)),
        flag: true,
    };
    let copied = pair;
    let encoded = rasn::uper::encode(&copied).unwrap();
    assert_eq!(
        rasn::uper::decode::<aaa_user::Pair>(&encoded).unwrap(),
        pair
    );
}

#[test]
fn derives_copy_for_real_fields_with_compatible_derives() {
    let output = RasnCompiler::new_with_config(RasnConfig {
        type_annotations: vec![],
        ..RasnConfig::default()
    })
    .add_asn_literal("TestModule DEFINITIONS ::= BEGIN RealRecord ::= SEQUENCE { sample REAL } END")
    .compile_to_string()
    .unwrap();

    assert!(output.warnings.is_empty());
    assert!(output.generated.contains("PartialEq, Copy)]"));
    assert!(output.generated.contains("pub sample: f64"));
}

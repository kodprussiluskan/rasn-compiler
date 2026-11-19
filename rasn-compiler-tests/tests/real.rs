use rasn_compiler_derive::asn1;

asn1!(
    r#"
    Numbers DEFINITIONS AUTOMATIC TAGS ::= BEGIN
        Single ::= REAL (0 | MINUS-INFINITY | PLUS-INFINITY | NOT-A-NUMBER | WITH COMPONENTS {
            mantissa (-16777215..16777215), base (2), exponent (-149..104)
        })
        Double ::= REAL (0 | MINUS-INFINITY | PLUS-INFINITY | NOT-A-NUMBER | WITH COMPONENTS {
            mantissa (-9007199254740991..9007199254740991), base (2), exponent (-1074..971)
        })
        Named ::= SEQUENCE OF Single
        Inline ::= SEQUENCE OF REAL (0 | MINUS-INFINITY | PLUS-INFINITY | NOT-A-NUMBER | WITH COMPONENTS {
            mantissa (-16777215..16777215), base (2), exponent (-149..104)
        })
    END

    Log DEFINITIONS AUTOMATIC TAGS ::= BEGIN
        IMPORTS Double FROM Numbers;
        Samples ::= SEQUENCE {
            named SEQUENCE OF Double,
            inline SEQUENCE OF REAL (0 | MINUS-INFINITY | PLUS-INFINITY | NOT-A-NUMBER | WITH COMPONENTS {
                mantissa (-9007199254740991..9007199254740991), base (2), exponent (-1074..971)
            })
        }
    END
    "#
);

#[test]
fn named_and_inline_binary32_have_identical_oer_and_jer() {
    let values: Vec<f32> = vec![1.5, -2.0];
    let named = numbers::Named(values.clone());
    let inline = numbers::Inline(values);
    let bytes = [1, 2, 0x3f, 0xc0, 0, 0, 0xc0, 0, 0, 0];
    assert_eq!(rasn::oer::encode(&named).unwrap(), bytes);
    assert_eq!(rasn::oer::encode(&inline).unwrap(), bytes);
    assert_eq!(rasn::oer::decode::<numbers::Named>(&bytes).unwrap(), named);
    assert_eq!(
        rasn::oer::decode::<numbers::Inline>(&bytes).unwrap(),
        inline
    );
    let json = "[1.5,-2.0]";
    assert_eq!(rasn::jer::encode(&named).unwrap(), json);
    assert_eq!(rasn::jer::encode(&inline).unwrap(), json);
    assert_eq!(rasn::jer::decode::<numbers::Named>(json).unwrap(), named);
    assert_eq!(rasn::jer::decode::<numbers::Inline>(json).unwrap(), inline);
}

#[test]
fn imported_and_inline_binary64_are_primitive_vectors() {
    let named: Vec<f64> = vec![1.5];
    let inline: Vec<f64> = vec![1.5];
    let samples = log::Samples { named, inline };
    let single = [1, 1, 0x3f, 0xf8, 0, 0, 0, 0, 0, 0];
    let bytes: Vec<_> = single.into_iter().chain(single).collect();
    assert_eq!(rasn::oer::encode(&samples).unwrap(), bytes);
    assert_eq!(rasn::oer::decode::<log::Samples>(&bytes).unwrap(), samples);
    let json = r#"{"inline":[1.5],"named":[1.5]}"#;
    assert_eq!(rasn::jer::encode(&samples).unwrap(), json);
    assert_eq!(rasn::jer::decode::<log::Samples>(json).unwrap(), samples);
}

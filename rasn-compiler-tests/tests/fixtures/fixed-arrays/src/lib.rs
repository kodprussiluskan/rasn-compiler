pub mod arrays {
    include!(concat!(env!("OUT_DIR"), "/arrays.rs"));
}
pub mod vectors {
    include!(concat!(env!("OUT_DIR"), "/vectors.rs"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constructs_named_and_empty_collections() {
        let _: &[arrays::collections::AnonymousFlags; 3] = &arrays::collections::FLAGS.0;
        let _: &[arrays::collections::Flag; 3] = &arrays::collections::NAMED_FLAGS.0;
        let _: &[arrays::collections::AnonymousEmpty; 0] = &arrays::collections::EMPTY.0;
        assert_eq!(*arrays::collections::FLAGS, *arrays::collections::COPIED);
        assert_eq!(arrays::collections::FLAGS.0[0].0, true);
        assert_eq!(arrays::collections::FLAGS.0[1].0, false);
        assert_eq!(arrays::collections::NAMED_FLAGS.0[2].0, true);
        assert!(arrays::collections::EMPTY.0.is_empty());
        assert_eq!(arrays::consumer::IMPORTED.0[0].0, true);
        let defaults = arrays::consumer::Defaults::default();
        let _: arrays::origin::Flag = defaults.local.clone();
        assert_eq!(defaults.local.0, 7);
        assert_eq!(defaults.flags.0[0].0, true);
    }

    #[test]
    fn constructs_nested_choice_and_default_values() {
        assert_eq!(arrays::collections::MATRIX.0[0].0[0].0, true);
        assert_eq!(arrays::collections::MATRIX.0[0].0[1].0, false);
        assert!(arrays::collections::RECORD.optional.is_none());
        assert_eq!(
            arrays::collections::PRESENT.optional.as_ref().unwrap()[0],
            true
        );
        let arrays = rasn::der::encode(&*arrays::collections::RECORD).unwrap();
        let vectors = rasn::der::encode(&*vectors::collections::RECORD).unwrap();
        assert_eq!(arrays, vectors);
        let decoded: arrays::collections::Record = rasn::der::decode(&arrays).unwrap();
        assert_eq!(decoded, *arrays::collections::RECORD);
        let arrays = rasn::der::encode(&*arrays::collections::PICK).unwrap();
        let vectors = rasn::der::encode(&*vectors::collections::PICK).unwrap();
        assert_eq!(arrays, vectors);
    }

    #[test]
    fn uses_underlying_wrapper_for_alias_value() {
        let _: &rasn::types::SequenceOf<arrays::collections::Flag> =
            &arrays::collections::THREE.0 .0;
        let _: &[arrays::collections::Flag; 3] = &arrays::collections::FIXED_ALIAS.0 .0;
        assert_eq!(arrays::collections::THREE.0 .0.len(), 3);
        assert_eq!(arrays::collections::FIXED_ALIAS.0 .0[1].0, false);
    }

    #[test]
    fn preserves_omission_of_absent_optional_scalar_fields() {
        assert_eq!(arrays::collections::SCALAR_RECORD.enabled, None);
        assert_eq!(vectors::collections::SCALAR_RECORD.enabled, None);
    }
}

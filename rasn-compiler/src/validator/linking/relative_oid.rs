use std::collections::BTreeSet;

use crate::intermediate::{
    error::{GrammarError, GrammarErrorType},
    ASN1Type, ASN1Value, ObjectIdentifierArc, ObjectIdentifierValue, ToplevelDefinition,
};

use super::Scope;

pub(super) fn resolve(
    oid: &ObjectIdentifierValue,
    scope: &Scope<'_>,
) -> Result<ObjectIdentifierValue, GrammarError> {
    let mut numbers = Vec::new();
    append_arcs(oid, scope, &mut BTreeSet::new(), &mut numbers)?;
    Ok(ObjectIdentifierValue(numbers))
}

fn append_arcs(
    oid: &ObjectIdentifierValue,
    scope: &Scope<'_>,
    active: &mut BTreeSet<(String, String)>,
    numbers: &mut Vec<ObjectIdentifierArc>,
) -> Result<(), GrammarError> {
    for arc in &oid.0 {
        if let Some(number) = arc.number {
            if u32::try_from(number).is_err() {
                return Err(error("RELATIVE-OID arc exceeds u32"));
            }
            numbers.push(number.into());
            continue;
        }
        let name = arc
            .name
            .as_deref()
            .ok_or_else(|| error("Missing RELATIVE-OID arc"))?;
        let Some(ToplevelDefinition::Value(value)) = scope.get(name) else {
            return Err(error(&format!(
                "Could not resolve RELATIVE-OID arc '{name}' in module '{}'", scope.module
            )));
        };
        let source = scope.for_definition(value.module_header.as_ref());
        let key = (source.module.to_owned(), name.to_owned());
        if !active.insert(key.clone()) {
            return Err(error(&format!(
                "Cyclic RELATIVE-OID arc reference '{name}'"
            )));
        }
        let ty = match &value.associated_type {
            ASN1Type::ElsewhereDeclaredType(reference) => reference.root(&source)?,
            other => other,
        };
        let mut initializer = &value.value;
        while let ASN1Value::LinkedNestedValue { value, .. } = initializer {
            initializer = value;
        }
        match (ty, initializer) {
            (
                ASN1Type::RelativeOid(_),
                ASN1Value::ObjectIdentifier(oid) | ASN1Value::RelativeOid(oid),
            ) => {
                append_arcs(oid, &source, active, numbers)?;
            }
            (
                ASN1Type::Integer(_),
                ASN1Value::Integer(number) | ASN1Value::LinkedIntValue { value: number, .. },
            ) => {
                let number = u32::try_from(*number)
                    .map_err(|_| error("RELATIVE-OID arc is outside the u32 range"))?;
                numbers.push(u128::from(number).into());
            }
            _ => {
                return Err(error(&format!(
                    "RELATIVE-OID arc '{name}' must be an integer or relative identifier value"
                )))
            }
        }
        active.remove(&key);
    }
    Ok(())
}

fn error(message: &str) -> GrammarError {
    GrammarError::new(message, GrammarErrorType::LinkerError)
}

use proc_macro2::{Literal, TokenStream};
use std::collections::BTreeSet;
use quote::quote;

use crate::generator::error::{GeneratorError, GeneratorErrorType};
use crate::intermediate::{
    constraints::{
        ComponentPresence, Constraint, ElementOrSetOperation, ElementSetSpecs, InnerTypeConstraint,
        SetOperator, SubtypeElements,
    },
    types::Real,
    ASN1Type, ASN1Value, ToplevelValueDefinition,
};

use super::{copy::resolve, Rasn, TypeDeclarations};

impl Rasn {
    /// Resolve the primitive and the Rust tuple wrappers, from outermost to innermost.
    pub(super) fn real_constant_type<'a>(
        &self,
        value: &'a ToplevelValueDefinition,
        declarations: &TypeDeclarations<'a>,
    ) -> Option<(&'a Real, Vec<TokenStream>)> {
        if let ASN1Type::Real(real) = &value.associated_type {
            return Some((real, vec![]));
        }
        let mut module = value.module_header.as_ref()?.borrow();
        let origin = module.name.clone();
        let mut ty = &value.associated_type;
        let mut wrappers = vec![];
        let mut visited = BTreeSet::new();
        while let ASN1Type::ElsewhereDeclaredType(reference) = ty {
            let id = resolve(reference, &module, |id| declarations.contains_key(id))?;
            let declaration = declarations.get(&id)?;
            if !visited.insert(id.clone()) {
                return None;
            }
            let wraps = !matches!(declaration.ty, ASN1Type::Real(_))
                || declaration.tag.is_some()
                || !declaration.jer.is_empty();
            if wraps {
                wrappers.push(self.to_rust_qualified_type(
                    (id.0 != origin).then_some(id.0.as_str()),
                    &id.1,
                ));
            }
            ty = &declaration.ty;
            if let ASN1Type::Real(real) = ty {
                return Some((real, wrappers));
            }
            module = declaration.module_header.as_ref()?.borrow();
        }
        None
    }

    pub(super) fn real_literal(value: f64) -> TokenStream {
        if value.is_nan() {
            quote!(core::primitive::f64::NAN)
        } else if value == f64::INFINITY {
            quote!(core::primitive::f64::INFINITY)
        } else if value == f64::NEG_INFINITY {
            quote!(core::primitive::f64::NEG_INFINITY)
        } else {
            let literal = Literal::f64_suffixed(value);
            quote!(#literal)
        }
    }

    pub(super) fn real_storage(real: &Real) -> Result<TokenStream, GeneratorError> {
        let storage = (|| {
            let [Constraint::Subtype(ElementSetSpecs {
                set,
                extensible: false,
            })] = real.constraints.as_slice()
            else {
                return None;
            };
            let mut components = None;
            real_components(set, &mut components)?;
            let components = components?;
            if components.constraints.len() != 3 {
                return None;
            }
            let mantissa = component_range(components, "mantissa")?;
            let exponent = component_range(components, "exponent")?;
            if component_range(components, "base")? != (2, 2) {
                return None;
            }
            // X.696 12.2 and 12.3 select the IEEE format from the effective bounds.
            if mantissa.0 >= -16_777_215
                && mantissa.1 <= 16_777_215
                && exponent.0 >= -149
                && exponent.1 <= 104
            {
                Some(quote!(f32))
            } else if mantissa.0 >= -9_007_199_254_740_991
                && mantissa.1 <= 9_007_199_254_740_991
                && exponent.0 >= -1074
                && exponent.1 <= 971
            {
                Some(quote!(f64))
            } else {
                None
            }
        })();
        storage.ok_or_else(|| GeneratorError {
            kind: GeneratorErrorType::NotYetInplemented,
            details:
                "REAL requires a base-2 WITH COMPONENTS constraint fitting OER binary32 or binary64"
                    .into(),
            top_level_declaration: None,
        })
    }
}

fn real_components<'a>(
    set: &'a ElementOrSetOperation,
    components: &mut Option<&'a InnerTypeConstraint>,
) -> Option<()> {
    match set {
        ElementOrSetOperation::Element(element) => real_component_set(element, components),
        ElementOrSetOperation::SetOperation(operation)
            if operation.operator == SetOperator::Union =>
        {
            real_component_set(&operation.base, components)?;
            real_components(&operation.operant, components)
        }
        _ => None,
    }
}

fn real_component_set<'a>(
    element: &'a SubtypeElements,
    components: &mut Option<&'a InnerTypeConstraint>,
) -> Option<()> {
    match element {
        SubtypeElements::MultipleTypeConstraints(inner) if components.is_none() => {
            *components = Some(inner);
            Some(())
        }
        SubtypeElements::SingleValue {
            value: ASN1Value::Integer(0),
            extensible: false,
        } => Some(()),
        SubtypeElements::SingleValue {
            value: ASN1Value::Real(value),
            extensible: false,
        } if *value == 0.0 || !value.is_finite() => Some(()),
        _ => None,
    }
}

fn component_range(components: &InnerTypeConstraint, name: &str) -> Option<(i128, i128)> {
    let component = components
        .constraints
        .iter()
        .find(|field| field.identifier == name)?;
    if component.presence != ComponentPresence::Unspecified {
        return None;
    }
    let [Constraint::Subtype(ElementSetSpecs {
        set,
        extensible: false,
    })] = component.constraints.as_slice()
    else {
        return None;
    };
    match set {
        ElementOrSetOperation::Element(SubtypeElements::SingleValue {
            value: ASN1Value::Integer(value),
            extensible: false,
        }) => Some((*value, *value)),
        ElementOrSetOperation::Element(SubtypeElements::ValueRange {
            min: Some(ASN1Value::Integer(min)),
            max: Some(ASN1Value::Integer(max)),
            extensible: false,
        }) if min <= max => Some((*min, *max)),
        _ => None,
    }
}

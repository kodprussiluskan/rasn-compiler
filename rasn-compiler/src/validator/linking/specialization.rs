use super::*;
use crate::intermediate::{
    constraints::{ElementOrSetOperation, SetOperator, SubtypeElements},
    parameterization::{ParameterGovernor, Parameterization, ParameterizationArgument},
};

impl ASN1Type {
    pub(in crate::validator) fn resolve_parameters(
        reference: &DeclarationElsewhere,
        tlds: &Scope<'_>,
        args: &[Parameter],
    ) -> Result<ASN1Type, GrammarError> {
        let identifier = &reference.identifier;
        match tlds.get_template(reference)? {
            template @ ToplevelDefinition::Type(ToplevelTypeDefinition {
                ty,
                tag,
                module_header,
                parameterization: Some(Parameterization { parameters }),
                ..
            }) => {
                if args.len() != parameters.len() {
                    return Err(grammar_error!(
                        LinkerError,
                        "Template {identifier} expected {} arguments, received {}",
                        parameters.len(),
                        args.len()
                    ));
                }
                let expansion = tlds.expansion_for(template).ok_or_else(|| {
                    grammar_error!(
                        LinkerError,
                        "Recursive specialization cycle at {identifier}"
                    )
                })?;
                let caller_scope = tlds.for_actual();
                let template_scope = caller_scope
                    .for_definition(module_header.as_ref())
                    .with_expansion(&expansion);
                let mut impl_template = ty.clone();
                let mut arguments = BTreeMap::new();
                let mut table_constraint_replacements = BTreeMap::new();
                for (
                    ParameterizationArgument {
                        dummy_reference,
                        param_governor,
                    },
                    arg,
                ) in parameters.iter().zip(args)
                {
                    match (arg, param_governor) {
                        (Parameter::ValueParameter(v), ParameterGovernor::TypeOrClass(gov)) => {
                            let mut value = v.clone();
                            value.resolve_specialization_value(&caller_scope)?;
                            value.check_specialization_governor(gov, &template_scope)?;
                            arguments.insert(
                                dummy_reference.clone(),
                                ToplevelDefinition::Value(ToplevelValueDefinition::from((
                                    dummy_reference.as_str(),
                                    value,
                                    gov.clone(),
                                ))),
                            );
                        }
                        (
                            Parameter::TypeParameter(t) | Parameter::TaggedTypeParameter(_, t),
                            ParameterGovernor::None,
                        ) => {
                            let mut actual = t.clone();
                            if let Some(replacement) =
                                actual.link_constraint_reference(dummy_reference, &caller_scope)?
                            {
                                actual = replacement;
                            }
                            if let Parameter::TaggedTypeParameter(tag, _) = arg {
                                let tag = AsnTag {
                                    environment: &caller_scope.tagging_environment()
                                        + &tag.environment,
                                    ..tag.clone()
                                };
                                actual = caller_scope.materialize_type(actual, Some(tag), None)?;
                            }
                            arguments.insert(
                                dummy_reference.clone(),
                                ToplevelDefinition::Type(ToplevelTypeDefinition::from((
                                    dummy_reference.as_str(),
                                    actual,
                                ))),
                            );
                        }
                        (Parameter::InformationObjectParameter(_), _) => {
                            return Err(grammar_error!(LinkerError, "Unsupported information-object actual for parameter {dummy_reference} of {identifier}"));
                        }
                        (Parameter::ObjectSetParameter(o), ParameterGovernor::Class(c)) => {
                            match &o.values.first() {
                                    Some(osv) if o.values.len() == 1 => {
                                        #[allow(suspicious_double_ref_op)]
                                        table_constraint_replacements.insert(dummy_reference, osv.clone());
                                    }
                                    _ => return Err(grammar_error!(LinkerError, "Expected object set value argument to contain single object set value!"))
                                }
                            let mut info = ASN1Information::ObjectSet(o.clone());
                            info.link_object_set_reference(tlds);
                            let mut tld = ToplevelInformationDefinition::from((
                                dummy_reference.as_str(),
                                info,
                                c.as_str(),
                            ));
                            tld.resolve_class_reference(&template_scope);
                            arguments
                                .insert(dummy_reference.clone(), ToplevelDefinition::Object(tld));
                        }
                        _ => {
                            return Err(grammar_error!(
                            LinkerError,
                            "Mismatching argument for parameter {dummy_reference} of {identifier}"
                        ))
                        }
                    }
                }
                let impl_tlds = template_scope.with_parameters(&arguments);
                if let Some(replacement) =
                    impl_template.link_constraint_reference(identifier, &impl_tlds)?
                {
                    impl_template = replacement;
                };
                impl_template
                    .collect_supertypes(&impl_tlds)
                    .or_else(|_| impl_template.collect_supertypes(&template_scope))?;
                for (dummy_reference, osv) in table_constraint_replacements {
                    impl_template.reassign_table_constraint(dummy_reference, osv)?;
                }
                for constraint in reference
                    .constraints
                    .iter()
                    .filter(|c| !matches!(c, Constraint::Parameter(_)))
                {
                    let mut constraint = constraint.clone();
                    constraint.link_cross_reference(identifier, &caller_scope)?;
                    let constraints = impl_template.constraints_mut().ok_or_else(|| {
                        grammar_error!(
                            LinkerError,
                            "Cannot apply subtype constraint to specialization of {identifier}"
                        )
                    })?;
                    constraints.push(constraint);
                }
                match tag {
                    Some(tag) => {
                        let mut tag = Some(tag.clone());
                        specialize_component_tag(&mut tag, ty, &impl_tlds, false, 0);
                        caller_scope.materialize_type(impl_template, tag, None)
                    }
                    None => Ok(impl_template),
                }
            }
            _ => Err(grammar_error!(
                LinkerError,
                "Failed to resolve supertype {identifier} of parameterized implementation."
            )),
        }
    }
}

pub(super) fn needs_explicit_specialization_tag(ty: &ASN1Type, scope: &Scope<'_>) -> bool {
    matches!(ty, ASN1Type::Choice(_) | ASN1Type::Any)
        || matches!(ty, ASN1Type::ElsewhereDeclaredType(reference)
            if reference.module.is_none() && scope.parameter(&reference.identifier).is_some())
}

pub(super) fn specialize_component_tag(
    tag: &mut Option<AsnTag>,
    ty: &ASN1Type,
    scope: &Scope<'_>,
    automatic: bool,
    index: usize,
) {
    let must_be_explicit = needs_explicit_specialization_tag(ty, scope);
    if let Some(tag) = tag {
        tag.environment = if tag.environment == TaggingEnvironment::Automatic && must_be_explicit {
            TaggingEnvironment::Explicit
        } else {
            &scope.tagging_environment() + &tag.environment
        };
    } else if automatic {
        *tag = Some(AsnTag {
            environment: if must_be_explicit {
                TaggingEnvironment::Explicit
            } else {
                TaggingEnvironment::Implicit
            },
            tag_class: TagClass::ContextSpecific,
            id: index as u64,
        });
    }
}

fn integer_governor_accepts(constraint: &ElementOrSetOperation, integer: i128) -> Option<bool> {
    fn contains(element: &SubtypeElements, integer: i128) -> Option<bool> {
        match element {
            SubtypeElements::SingleValue {
                value: ASN1Value::Integer(value),
                extensible,
            } => Some(*extensible || integer == *value),
            SubtypeElements::ValueRange {
                min,
                max,
                extensible,
            } => {
                let lower = match min {
                    Some(ASN1Value::Integer(value)) => integer >= *value,
                    None => true,
                    _ => return None,
                };
                let upper = match max {
                    Some(ASN1Value::Integer(value)) => integer <= *value,
                    None => true,
                    _ => return None,
                };
                Some(*extensible || (lower && upper))
            }
            SubtypeElements::ContainedSubtype {
                subtype: ASN1Type::Integer(value),
                extensible,
            } => {
                if *extensible {
                    return Some(true);
                }
                value
                    .constraints
                    .iter()
                    .try_fold(true, |accepted, constraint| {
                        let Constraint::Subtype(constraint) = constraint else {
                            return None;
                        };
                        Some(
                            accepted
                                && (constraint.extensible
                                    || integer_governor_accepts(&constraint.set, integer)?),
                        )
                    })
            }
            _ => None,
        }
    }

    match constraint {
        ElementOrSetOperation::Element(element) => contains(element, integer),
        ElementOrSetOperation::SetOperation(operation) => {
            let lhs = contains(&operation.base, integer)?;
            let rhs = integer_governor_accepts(&operation.operant, integer)?;
            Some(match operation.operator {
                SetOperator::Intersection => lhs && rhs,
                SetOperator::Union => lhs || rhs,
                SetOperator::Except => lhs && !rhs,
            })
        }
    }
}

impl ASN1Value {
    pub(super) fn resolve_specialization_value(
        &mut self,
        scope: &Scope<'_>,
    ) -> Result<(), GrammarError> {
        let mut visited = HashSet::new();
        let mut scope = *scope;
        loop {
            while let Self::LinkedNestedValue { value, .. } = self {
                *self = std::mem::replace(value.as_mut(), Self::Null);
            }
            if let Self::LinkedIntValue { value, .. } = self {
                *self = Self::Integer(*value);
            }
            let Self::ElsewhereDeclaredValue {
                module,
                identifier,
                parent: None,
            } = self
            else {
                break;
            };
            let Some(ToplevelDefinition::Value(value)) =
                scope.get_qualified(module.as_deref(), identifier)
            else {
                break;
            };
            if !visited.insert(std::ptr::from_ref(value)) {
                return Err(grammar_error!(
                    LinkerError,
                    "Value reference cycle at {identifier}"
                ));
            }
            scope = scope.for_definition(value.module_header.as_ref());
            *self = value.value.clone();
        }
        Ok(())
    }

    fn check_specialization_governor(
        &self,
        governor: &ASN1Type,
        scope: &Scope<'_>,
    ) -> Result<(), GrammarError> {
        let mut ty = governor;
        let mut scope = *scope;
        let mut seen = HashSet::new();
        let mut constraints = Vec::new();
        while let ASN1Type::ElsewhereDeclaredType(reference) = ty {
            for mut constraint in reference.constraints.iter().cloned() {
                constraint.link_cross_reference(&reference.identifier, &scope)?;
                constraints.push(constraint);
            }
            let Some(ToplevelDefinition::Type(definition)) = scope.get_type(reference) else {
                return Err(grammar_error!(
                    LinkerError,
                    "Cannot resolve governor {}",
                    reference.identifier
                ));
            };
            if !seen.insert(std::ptr::from_ref(definition)) {
                return Err(grammar_error!(
                    LinkerError,
                    "Governor alias cycle at {}",
                    reference.identifier
                ));
            }
            ty = &definition.ty;
            scope = scope.for_definition(definition.module_header.as_ref());
        }
        let compatible = match (ty, self) {
            (ASN1Type::Integer(integer), Self::Integer(value)) => {
                for mut constraint in integer.constraints.iter().cloned() {
                    constraint.link_cross_reference(&ty.as_str().into_owned(), &scope)?;
                    constraints.push(constraint);
                }
                let mut compatible = true;
                for constraint in &constraints {
                    let Constraint::Subtype(constraint) = constraint else {
                        return Err(grammar_error!(
                            LinkerError,
                            "Unsupported integer governor constraint: {constraint:?}"
                        ));
                    };
                    if !constraint.extensible {
                        compatible &= integer_governor_accepts(&constraint.set, *value)
                            .ok_or_else(|| {
                                grammar_error!(
                                    LinkerError,
                                    "Unsupported integer governor constraint: {constraint:?}"
                                )
                            })?;
                    }
                }
                compatible
            }
            (ASN1Type::Null, Self::Null)
            | (ASN1Type::Boolean(_), Self::Boolean(_))
            | (ASN1Type::Real(_), Self::Real(_) | Self::Integer(_))
            | (ASN1Type::CharacterString(_), Self::String(_))
            | (ASN1Type::BitString(_), Self::BitString(_) | Self::BitStringNamedBits(_))
            | (ASN1Type::OctetString(_), Self::OctetString(_) | Self::BitString(_))
            | (ASN1Type::ObjectIdentifier(_), Self::ObjectIdentifier(_))
            | (ASN1Type::RelativeOid(_), Self::RelativeOid(_))
            | (ASN1Type::Enumerated(_), Self::ElsewhereDeclaredValue { .. })
            | (
                ASN1Type::Sequence(_)
                | ASN1Type::Set(_)
                | ASN1Type::SequenceOf(_)
                | ASN1Type::SetOf(_),
                Self::SequenceOrSet(_),
            )
            | (ASN1Type::Choice(_), Self::Choice { .. }) => true,
            _ => false,
        };
        if compatible {
            Ok(())
        } else {
            Err(grammar_error!(
                LinkerError,
                "Actual {self:?} is incompatible with governor {}",
                governor.as_str()
            ))
        }
    }
}

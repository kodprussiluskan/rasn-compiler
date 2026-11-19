use std::collections::HashSet;

use super::{
    Typescript,
    template::typealias_template,
    utils::{format_comments, to_jer_identifier, type_to_tokens},
};
use crate::{
    generator::error::{GeneratorError, GeneratorErrorType},
    intermediate::{
        ASN1Type, ToplevelDefinition, ToplevelTypeDefinition,
        jer::{Representation, Resolved},
        types::Optionality,
    },
};

impl Typescript {
    pub(super) fn generate_with_jer(&self, declaration: &ToplevelTypeDefinition) -> Result<String, GeneratorError> {
        let mut effective = declaration.jer.clone();
        let base = self.follow(declaration, &mut effective)?;
        let wire_type = match &base.ty {
            ASN1Type::Enumerated(enumerated) => {
                let replacements = effective.text.as_deref().unwrap_or_default();
                enumerated
                    .members
                    .iter()
                    .map(|member| format!("{:?}", member_name(replacements, &member.name)))
                    .collect::<Vec<_>>()
                    .join(" | ")
            }
            ASN1Type::Choice(choice) => {
                let unwrapped = effective.representation == Some(Representation::Unwrapped);
                choice
                    .options
                    .iter()
                    .map(|option| {
                        let ty = type_to_tokens(&option.ty);
                        if unwrapped {
                            ty
                        } else {
                            format!("{{ {:?}: {ty} }}", member_name(&effective.names, &option.name))
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" | ")
            }
            ASN1Type::Sequence(sequence) if effective.representation == Some(Representation::Array) => {
                let mut slots = Vec::new();
                self.array_slots(sequence, base, false, &mut slots)?;
                let last_required = slots.iter().rposition(|(_, required)| *required);
                let tuple = slots
                    .into_iter()
                    .enumerate()
                    .map(|(index, (ty, required))| {
                        if required {
                            ty
                        } else if last_required.is_some_and(|last| index <= last) {
                            format!("{ty} | null")
                        } else {
                            format!("({ty} | null)?")
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("[{tuple}]")
            }
            ASN1Type::Sequence(sequence) | ASN1Type::Set(sequence) => {
                let fields = sequence
                    .members
                    .iter()
                    .map(|member| {
                        let name = member_name(&effective.names, &member.name);
                        let optional = if matches!(member.optionality, Optionality::Required) {
                            ""
                        } else {
                            "?"
                        };
                        let ty = type_to_tokens(&member.ty);
                        format!("{name:?}{optional}: {ty}")
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{{ {fields} }}")
            }
            ASN1Type::SetOf(collection) if effective.representation == Some(Representation::Object) => {
                // TypeScript describes the shape after parsing JSON. The wire format may
                // contain more than one occurrence of the same property name.
                let element = self.follow_type(&collection.element_type, base)?;
                let ASN1Type::Sequence(pair) = element else {
                    return Err(error(declaration, "JER OBJECT element type cannot be resolved"));
                };
                let value = type_to_tokens(&pair.members[1].ty);
                format!("{{ [key: string]: {value} }}")
            }
            ty => type_to_tokens(ty),
        };
        let comments = if effective.representation == Some(Representation::Object) {
            format!(
                "{}\n/** JER OBJECT may repeat member names in the JSON wire representation. */",
                format_comments(&declaration.comments)
            )
        } else {
            format_comments(&declaration.comments)
        };
        Ok(typealias_template(
            &comments,
            &to_jer_identifier(&declaration.name),
            &wire_type,
        ))
    }

    pub(super) fn value_uses_jer(&self, value: &crate::intermediate::ToplevelValueDefinition) -> bool {
        if self.types.is_empty() {
            return false;
        }
        let header = value.module_header.as_ref().map(|header| header.borrow());
        self.type_uses_jer(&value.associated_type, header.as_deref(), &mut HashSet::new())
    }

    fn type_uses_jer(
        &self,
        ty: &ASN1Type,
        header: Option<&crate::intermediate::ModuleHeader>,
        seen: &mut HashSet<(String, String)>,
    ) -> bool {
        match ty {
            ASN1Type::Jer(_, _) => true,
            ASN1Type::ElsewhereDeclaredType(reference) => {
                let key = reference_key(reference, header);
                if !seen.insert(key.clone()) {
                    return false;
                }
                self.types.get(&key).is_some_and(|definition| {
                    let header = definition.module_header.as_ref().map(|header| header.borrow());
                    !definition.jer.is_empty() || self.type_uses_jer(&definition.ty, header.as_deref(), seen)
                })
            }
            ASN1Type::Sequence(seq) | ASN1Type::Set(seq) => seq
                .members
                .iter()
                .any(|member| self.type_uses_jer(&member.ty, header, seen)),
            ASN1Type::Choice(choice) => choice
                .options
                .iter()
                .any(|option| self.type_uses_jer(&option.ty, header, seen)),
            ASN1Type::SequenceOf(list) | ASN1Type::SetOf(list) => self.type_uses_jer(&list.element_type, header, seen),
            _ => false,
        }
    }

    fn array_slots(
        &self,
        sequence: &crate::intermediate::types::SequenceOrSet,
        owner: &ToplevelTypeDefinition,
        group: bool,
        output: &mut Vec<(String, bool)>,
    ) -> Result<(), GeneratorError> {
        for (index, member) in sequence.members.iter().enumerate() {
            let extension = sequence.extensible.is_some_and(|root| index >= root);
            if extension
                && member
                    .name
                    .starts_with(crate::common::INTERNAL_EXTENSION_GROUP_NAME_PREFIX)
            {
                let ty = self.follow_type(&member.ty, owner)?;
                if let ASN1Type::Sequence(inner) = ty {
                    self.array_slots(inner, owner, true, output)?;
                    continue;
                }
            }
            output.push((
                type_to_tokens(&member.ty),
                !group && !extension && matches!(member.optionality, Optionality::Required),
            ));
        }
        Ok(())
    }

    fn follow<'a>(
        &'a self,
        mut definition: &'a ToplevelTypeDefinition,
        effective: &mut Resolved,
    ) -> Result<&'a ToplevelTypeDefinition, GeneratorError> {
        let mut seen = HashSet::new();
        while let ASN1Type::ElsewhereDeclaredType(reference) = &definition.ty {
            if !seen.insert(definition as *const _) {
                return Err(error(definition, "cyclic JER type reference"));
            }
            definition = self.lookup(reference, definition)?;
            if effective.representation.is_none() {
                effective.representation = definition.jer.representation;
            }
            if effective.text.is_none() {
                effective.text = definition.jer.text.clone();
            }
            if effective.names.is_empty() {
                effective.names = definition.jer.names.clone();
            }
        }
        Ok(definition)
    }

    fn follow_type<'a>(
        &'a self,
        ty: &'a ASN1Type,
        owner: &'a ToplevelTypeDefinition,
    ) -> Result<&'a ASN1Type, GeneratorError> {
        if let ASN1Type::ElsewhereDeclaredType(reference) = ty {
            let definition = self.lookup(reference, owner)?;
            Ok(&self.follow(definition, &mut Resolved::default())?.ty)
        } else {
            Ok(ty)
        }
    }

    fn lookup<'a>(
        &'a self,
        reference: &crate::intermediate::DeclarationElsewhere,
        owner: &ToplevelTypeDefinition,
    ) -> Result<&'a ToplevelTypeDefinition, GeneratorError> {
        let header = owner.module_header.as_ref().map(|header| header.borrow());
        let key = reference_key(reference, header.as_deref());
        self.types.get(&key).ok_or_else(|| {
            error(
                owner,
                &format!("JER type {} could not be resolved", reference.identifier),
            )
        })
    }
}

fn reference_key(
    reference: &crate::intermediate::DeclarationElsewhere,
    header: Option<&crate::intermediate::ModuleHeader>,
) -> (String, String) {
    let module = reference.module.as_deref().or_else(|| {
        header.map(|header| {
            header
                .imports
                .iter()
                .find(|import| import.types.contains(&reference.identifier))
                .map_or(header.name.as_str(), |import| {
                    import.global_module_reference.module_reference.as_str()
                })
        })
    });
    (module.unwrap_or_default().to_owned(), reference.identifier.clone())
}

fn member_name<'a>(map: &'a [(String, String)], original: &'a str) -> &'a str {
    map.iter()
        .find_map(|(before, after)| (before == original).then_some(after.as_str()))
        .unwrap_or(original)
}

fn error(declaration: &ToplevelTypeDefinition, details: &str) -> GeneratorError {
    GeneratorError::new(
        Some(ToplevelDefinition::Type(declaration.clone())),
        details,
        GeneratorErrorType::Asn1TypeMismatch,
    )
}

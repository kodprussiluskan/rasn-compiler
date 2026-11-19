use proc_macro2::TokenStream;
use quote::quote;

use crate::intermediate::{types::*, ASN1Type, ASN1Value, ModuleHeader};

use super::{copy, Rasn};
use crate::generator::error::{GeneratorError, GeneratorErrorType};

impl Rasn {
    pub(super) fn contains_collection_value(value: &ASN1Value) -> bool {
        match value {
            ASN1Value::LinkedArrayLikeValue(_) => true,
            ASN1Value::LinkedNestedValue { value, .. }
            | ASN1Value::Choice {
                inner_value: value, ..
            } => Self::contains_collection_value(value),
            ASN1Value::LinkedStructLikeValue(fields) => fields
                .iter()
                .any(|(_, _, field)| Self::contains_collection_value(field.value())),
            _ => false,
        }
    }

    pub(super) fn requires_typed_value(
        &self,
        ty: &ASN1Type,
        header: Option<&ModuleHeader>,
        visited: &mut Vec<(String, String)>,
    ) -> bool {
        match ty {
            ASN1Type::SequenceOf(_) | ASN1Type::SetOf(_) => true,
            ASN1Type::Jer(_, ty) => self.requires_typed_value(ty, header, visited),
            ASN1Type::Sequence(sequence) | ASN1Type::Set(sequence) => {
                sequence.members.iter().any(|member| {
                    matches!(member.optionality, Optionality::Optional)
                        || self.requires_typed_value(&member.ty, header, visited)
                })
            }
            ASN1Type::Choice(choice) => choice
                .options
                .iter()
                .any(|option| self.requires_typed_value(&option.ty, header, visited)),
            ASN1Type::ElsewhereDeclaredType(reference) => {
                let id = header.and_then(|header| {
                    copy::resolve(reference, header, |id| self.value_types.contains_key(id))
                });
                let Some(id) = id else { return false };
                if visited.contains(&id) {
                    return false;
                }
                visited.push(id.clone());
                let declaration = &self.value_types[&id];
                let header = declaration
                    .module_header
                    .as_ref()
                    .map(|header| header.borrow());
                let found = self.requires_typed_value(&declaration.ty, header.as_deref(), visited);
                visited.pop();
                found
            }
            _ => false,
        }
    }

    fn value_constructor(&self, name: &str, header: Option<&ModuleHeader>) -> TokenStream {
        let module = header.filter(|header| {
            self.copy_module
                .as_ref()
                .is_some_and(|current| current.name != header.name)
        });
        self.to_rust_qualified_type(module.map(|header| header.name.as_str()), name)
    }

    pub(super) fn typed_value(
        &self,
        value: &ASN1Value,
        ty: &ASN1Type,
        name: Option<&str>,
        header: Option<&ModuleHeader>,
        owner: Option<(&str, &str)>,
    ) -> Result<TokenStream, GeneratorError> {
        if let ASN1Value::LinkedElsewhereDefinedValue { can_be_const, .. } = value {
            let source = self.value_to_tokens(value, None)?;
            return Ok(if *can_be_const {
                source
            } else {
                quote!((*#source).clone())
            });
        }
        let value = match value {
            ASN1Value::LinkedNestedValue { value, .. } => value.as_ref(),
            value => value,
        };
        if let ASN1Type::ElsewhereDeclaredType(reference) = ty {
            if let ASN1Value::LinkedArrayLikeValue(values) = value {
                if super::storage::fixed_count(&reference.constraints)?
                    .is_some_and(|count| values.len() != count as usize)
                {
                    return Err(GeneratorError::new(
                        None,
                        "Collection value length does not match its fixed size",
                        GeneratorErrorType::Asn1TypeMismatch,
                    ));
                }
            }
            let id = header.and_then(|header| {
                copy::resolve(reference, header, |id| self.value_types.contains_key(id))
            });
            let declaration = id.as_ref().and_then(|id| self.value_types.get(id));
            if let Some(declaration) = declaration {
                let own_header = declaration
                    .module_header
                    .as_ref()
                    .map(|header| header.borrow());
                let id = id.as_ref().unwrap();
                let inner = self.typed_value(
                    value,
                    &declaration.ty,
                    Some(&declaration.name),
                    own_header.as_deref(),
                    Some((&id.0, &id.1)),
                )?;
                if matches!(
                    declaration.ty,
                    ASN1Type::Sequence(_) | ASN1Type::Set(_) | ASN1Type::Choice(_)
                ) {
                    return Ok(inner);
                }
                let constructor =
                    self.to_rust_qualified_type(reference.module.as_deref(), &reference.identifier);
                return Ok(quote!(#constructor(#inner)));
            }
            return self.value_to_tokens(value, Some(&self.type_to_tokens(ty)?));
        }
        match (ty, value) {
            (ASN1Type::Jer(_, ty), _) => self.typed_value(value, ty, name, header, owner),
            (
                ASN1Type::SequenceOf(collection) | ASN1Type::SetOf(collection),
                ASN1Value::LinkedArrayLikeValue(values),
            ) => {
                let is_set = matches!(ty, ASN1Type::SetOf(_));
                let count = if is_set {
                    None
                } else {
                    self.collection_array_length_for(collection, owner)?
                };
                let fixed = super::storage::fixed_count(&collection.constraints)?;
                if fixed.is_some_and(|count| values.len() != count as usize) {
                    return Err(GeneratorError::new(
                        None,
                        "Collection value length does not match its fixed size",
                        GeneratorErrorType::Asn1TypeMismatch,
                    ));
                }
                let anonymous = name
                    .filter(|_| {
                        !matches!(
                            collection.element_type.as_ref(),
                            ASN1Type::ElsewhereDeclaredType(_)
                        ) && !(matches!(collection.element_type.as_ref(), ASN1Type::Real(_))
                            && !is_set
                            && collection.element_tag.is_none())
                    })
                    .map(|name| format!("Anonymous{}", self.to_rust_title_case(name)));
                let elements = values
                    .iter()
                    .map(|value| {
                        let tokens = self.typed_value(
                            value,
                            &collection.element_type,
                            anonymous.as_deref(),
                            header,
                            owner,
                        )?;
                        if let Some(anonymous) = &anonymous {
                            if !matches!(
                                collection.element_type.as_ref(),
                                ASN1Type::Sequence(_) | ASN1Type::Set(_) | ASN1Type::Choice(_)
                            ) {
                                let constructor = self.value_constructor(anonymous, header);
                                return Ok(quote!(#constructor(#tokens)));
                            }
                        }
                        Ok(tokens)
                    })
                    .collect::<Result<Vec<_>, GeneratorError>>()?;
                Ok(if count.is_some() {
                    quote!([#(#elements),*])
                } else if is_set {
                    quote!(SetOf::from(alloc::vec![#(#elements),*]))
                } else {
                    quote!(alloc::vec![#(#elements),*])
                })
            }
            (
                ASN1Type::Sequence(sequence) | ASN1Type::Set(sequence),
                ASN1Value::LinkedStructLikeValue(fields),
            ) => {
                let name = name.ok_or_else(|| {
                    GeneratorError::new(
                        None,
                        "Missing generated name for structured collection value",
                        GeneratorErrorType::Unidentified,
                    )
                })?;
                let constructor = self.value_constructor(name, header);
                let members = sequence
                    .members
                    .iter()
                    .map(|member| {
                        let field = fields.iter().find(|(field, _, _)| field == &member.name);
                        match field {
                            Some((_, _, value)) => {
                                let value = self.member_value(
                                    value.value(),
                                    &member.ty,
                                    &member.name,
                                    name,
                                    header,
                                    owner,
                                )?;
                                Ok(if matches!(member.optionality, Optionality::Optional) {
                                    quote!(Some(#value))
                                } else {
                                    value
                                })
                            }
                            None if matches!(member.optionality, Optionality::Optional) => {
                                Ok(quote!(None))
                            }
                            None => Err(GeneratorError::new(
                                None,
                                "Missing required structured value field",
                                GeneratorErrorType::Asn1TypeMismatch,
                            )),
                        }
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(quote!(#constructor::new(#(#members),*)))
            }
            (
                ASN1Type::Choice(choice),
                ASN1Value::Choice {
                    variant_name,
                    inner_value,
                    ..
                },
            ) => {
                let name = name.ok_or_else(|| {
                    GeneratorError::new(
                        None,
                        "Missing generated name for collection CHOICE value",
                        GeneratorErrorType::Unidentified,
                    )
                })?;
                let constructor = self.value_constructor(name, header);
                let option = choice
                    .options
                    .iter()
                    .find(|option| &option.name == variant_name)
                    .ok_or_else(|| {
                        GeneratorError::new(
                            None,
                            "Unknown CHOICE alternative",
                            GeneratorErrorType::Asn1TypeMismatch,
                        )
                    })?;
                let value =
                    self.member_value(inner_value, &option.ty, &option.name, name, header, owner)?;
                let variant = self.to_rust_enum_identifier(variant_name);
                Ok(quote!(#constructor::#variant(#value)))
            }
            _ => self.value_to_tokens(
                value,
                name.map(|name| self.to_rust_title_case(name)).as_ref(),
            ),
        }
    }

    pub(super) fn member_value(
        &self,
        value: &ASN1Value,
        ty: &ASN1Type,
        member: &str,
        parent: &str,
        header: Option<&ModuleHeader>,
        owner: Option<(&str, &str)>,
    ) -> Result<TokenStream, GeneratorError> {
        let name = Self::needs_unnesting(ty).then(|| self.inner_name(member, parent).to_string());
        let value = self.typed_value(value, ty, name.as_deref(), header, owner)?;
        if let Some(name) = name {
            if matches!(ty, ASN1Type::SequenceOf(_) | ASN1Type::SetOf(_)) {
                let constructor = self.value_constructor(&name, header);
                return Ok(quote!(#constructor(#value)));
            }
        }
        Ok(value)
    }
}

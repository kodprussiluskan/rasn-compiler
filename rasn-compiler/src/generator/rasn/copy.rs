use std::collections::{BTreeMap, BTreeSet};

use crate::intermediate::{
    types::MemberOrOption, ASN1Type, DeclarationElsewhere, ModuleHeader, ToplevelDefinition,
    ToplevelTypeDefinition,
};

use super::Rasn;

type TypeId = (String, String);
const COPY_BUILTINS: [&str; 4] = [
    "UtcTime",
    "GeneralizedTime",
    "FixedBitString",
    "FixedOctetString",
];

#[derive(Debug, Default)]
pub(super) struct CopyTypes {
    known: BTreeMap<TypeId, bool>,
    containing_real: BTreeSet<TypeId>,
    shadowed_builtins: BTreeMap<String, BTreeSet<&'static str>>,
}

impl CopyTypes {
    fn unshadowed_builtins(&self, module: Option<&str>) -> [bool; COPY_BUILTINS.len()] {
        let shadows = module.and_then(|module| self.shadowed_builtins.get(module));
        COPY_BUILTINS.map(|name| shadows.is_none_or(|names| !names.contains(name)))
    }
}

fn resolve(
    reference: &DeclarationElsewhere,
    module: &ModuleHeader,
    contains: impl Fn(&TypeId) -> bool,
) -> Option<TypeId> {
    let in_module = |name: &str| (name.to_owned(), reference.identifier.clone());
    if let Some(name) = &reference.module {
        return Some(in_module(name));
    }
    let local = in_module(&module.name);
    if contains(&local) {
        return Some(local);
    }

    let mut imports = module
        .imports
        .iter()
        .filter(|import| import.types.contains(&reference.identifier))
        .map(|import| in_module(&import.global_module_reference.module_reference));
    let first = imports.next()?;
    imports.all(|other| other == first).then_some(first)
}

impl Rasn {
    fn can_infer_copy(&self) -> bool {
        // Custom imports and attributes can change which Rust type a field refers to.
        self.config.custom_imports.is_empty() && self.config.type_annotations.is_empty()
    }

    pub(super) fn collect_copy_types(&self, modules: &[Vec<ToplevelDefinition>]) -> CopyTypes {
        let mut types = CopyTypes::default();
        let declarations = modules
            .iter()
            .flatten()
            .filter_map(|item| {
                let ToplevelDefinition::Type(ty) = item else {
                    return None;
                };
                let module = ty.module_header.as_ref()?.borrow().name.clone();
                Some(((module, ty.name.clone()), ty))
            })
            .collect::<BTreeMap<_, _>>();
        types.known.extend(declarations.keys().cloned().map(|id| (id, false)));
        loop {
            let mut changed = false;
            for (id, declaration) in &declarations {
                let header = declaration.module_header.as_ref().unwrap().borrow();
                if contains_real(&declaration.ty, &|reference| {
                    resolve(reference, &header, |id| declarations.contains_key(id))
                        .is_some_and(|id| types.containing_real.contains(&id))
                }) {
                    changed |= types.containing_real.insert(id.clone());
                }
            }
            if !changed {
                break;
            }
        }
        if !self.can_infer_copy() {
            return types;
        }
        types.known.clear();
        let headers: Vec<_> = modules
            .iter()
            .filter_map(|module| module.first()?.get_module_header())
            .collect();
        let mut builtin_names = headers
            .iter()
            .map(|header| (header.borrow().name.clone(), BTreeSet::new()))
            .collect::<BTreeMap<_, _>>();
        for ((module, name), ty) in &declarations {
            if ty.parameterization.is_none() {
                for runtime_name in COPY_BUILTINS {
                    if self.shadows_rust_name(name, &ty.ty, runtime_name) {
                        builtin_names
                            .entry(module.clone())
                            .or_default()
                            .insert(runtime_name);
                    }
                }
            }
        }
        for header in headers {
            let header = header.borrow();
            for name in COPY_BUILTINS {
                let shadowed = builtin_names[&header.name].contains(name)
                    || header.imports.iter().any(|import| {
                        if self.uses_wildcard_import(import) {
                            // An external module can export this name through a glob.
                            builtin_names
                                .get(&import.global_module_reference.module_reference)
                                .is_none_or(|names| names.contains(name))
                        } else {
                            import.types.iter().any(|item| {
                                item.starts_with(|c: char| c.is_uppercase())
                                    && self.to_rust_title_case(item).to_string() == name
                            })
                        }
                    });
                if shadowed {
                    types
                        .shadowed_builtins
                        .entry(header.name.clone())
                        .or_default()
                        .insert(name);
                }
            }
        }
        for id in declarations.keys() {
            self.evaluate_copy(id, &declarations, &mut types);
        }
        types
    }

    fn shadows_rust_name(&self, name: &str, ty: &ASN1Type, runtime_name: &str) -> bool {
        let name = self.to_rust_title_case(name).to_string();
        if name == runtime_name {
            return true;
        }
        if !runtime_name.starts_with(&name) {
            return false;
        }
        let shadows = |member: &str, ty| {
            Self::needs_unnesting(ty)
                && self.shadows_rust_name(
                    &self.inner_name(member, &name).to_string(),
                    ty,
                    runtime_name,
                )
        };
        match ty {
            ASN1Type::Sequence(seq) | ASN1Type::Set(seq) => seq
                .members
                .iter()
                .any(|member| shadows(&member.name, &member.ty)),
            ASN1Type::Choice(choice) => choice
                .options
                .iter()
                .any(|option| shadows(&option.name, &option.ty)),
            // Anonymous collection elements start with Anonymous and cannot shadow a runtime name.
            _ => false,
        }
    }

    fn evaluate_copy(
        &self,
        id: &TypeId,
        declarations: &BTreeMap<TypeId, &ToplevelTypeDefinition>,
        types: &mut CopyTypes,
    ) -> bool {
        if let Some(copy) = types.known.get(id) {
            return *copy;
        }
        let Some(ty) = declarations.get(id) else {
            return false;
        };

        // A dependency that reaches a declaration already being evaluated cannot prove Copy.
        types.known.insert(id.clone(), false);
        let copy = ty.parameterization.is_none()
            && ty.module_header.as_ref().is_some_and(|module| {
                let module = module.borrow();
                let builtins = types.unshadowed_builtins(Some(&module.name));
                self.type_is_copy(&ty.ty, false, builtins, &mut |reference| {
                    resolve(reference, &module, |id| declarations.contains_key(id))
                        .is_some_and(|id| self.evaluate_copy(&id, declarations, types))
                })
            });
        types.known.insert(id.clone(), copy);
        copy
    }

    pub(super) fn can_derive_copy(&self, ty: &ASN1Type) -> bool {
        let builtins = self
            .copy_types
            .unshadowed_builtins(self.copy_module.as_ref().map(|module| module.name.as_str()));
        self.can_infer_copy()
            && self.type_is_copy(ty, false, builtins, &mut |reference| {
                self.copy_module.as_ref().is_some_and(|module| {
                    resolve(reference, module, |id| {
                        self.copy_types.known.contains_key(id)
                    })
                    .is_some_and(|id| self.copy_types.known.get(&id) == Some(&true))
                })
            })
    }

    pub(super) fn contains_real(&self, ty: &ASN1Type) -> bool {
        contains_real(ty, &|reference| {
            self.copy_module.as_ref().is_some_and(|module| {
                resolve(reference, module, |id| self.copy_types.known.contains_key(id))
                    .is_some_and(|id| self.copy_types.containing_real.contains(&id))
            })
        })
    }

    fn type_is_copy(
        &self,
        ty: &ASN1Type,
        inline: bool,
        builtins: [bool; COPY_BUILTINS.len()],
        resolve: &mut impl FnMut(&DeclarationElsewhere) -> bool,
    ) -> bool {
        let [utc_time, generalized_time, fixed_bits, fixed_octets] = builtins;
        match ty {
            ASN1Type::Null | ASN1Type::Boolean(_) | ASN1Type::Enumerated(_) => true,
            ASN1Type::UTCTime(_) => utc_time,
            ASN1Type::GeneralizedTime(_) => generalized_time,
            ASN1Type::Real(real) => inline || Self::real_storage(real).is_ok(),
            ASN1Type::Integer(integer) if inline => self
                .inline_integer_type(integer)
                .is_ok_and(|storage| storage != "Integer"),
            ASN1Type::Integer(integer) => !integer.int_type().is_unbounded(),
            ASN1Type::BitString(bits) => !inline && fixed_bits && bits.fixed_size().is_some(),
            ASN1Type::OctetString(octets) => {
                !inline && fixed_octets && octets.fixed_size().is_some()
            }
            ASN1Type::ElsewhereDeclaredType(reference) => resolve(reference),
            ASN1Type::Sequence(seq) | ASN1Type::Set(seq) => seq
                .members
                .iter()
                .all(|member| self.member_is_copy(member, builtins, resolve)),
            ASN1Type::Choice(choice) => choice
                .options
                .iter()
                .all(|option| self.member_is_copy(option, builtins, resolve)),
            _ => false,
        }
    }

    fn member_is_copy(
        &self,
        member: &impl MemberOrOption,
        builtins: [bool; COPY_BUILTINS.len()],
        resolve: &mut impl FnMut(&DeclarationElsewhere) -> bool,
    ) -> bool {
        // Recursive named or anonymous fields use Box. Option never changes eligibility.
        !member.is_recursive()
            && self.type_is_copy(
                member.ty(),
                !Self::needs_unnesting(member.ty()),
                builtins,
                resolve,
            )
    }
}


fn contains_real(ty: &ASN1Type, resolve: &impl Fn(&DeclarationElsewhere) -> bool) -> bool {
    match ty {
        ASN1Type::Real(_) => true,
        ASN1Type::ElsewhereDeclaredType(reference) => resolve(reference),
        ASN1Type::Jer(_, ty) => contains_real(ty, resolve),
        ASN1Type::SequenceOf(list) | ASN1Type::SetOf(list) => contains_real(&list.element_type, resolve),
        ASN1Type::Sequence(sequence) | ASN1Type::Set(sequence) => sequence.members.iter().any(|member| contains_real(&member.ty, resolve)),
        ASN1Type::Choice(choice) => choice.options.iter().any(|option| contains_real(&option.ty, resolve)),
        _ => false,
    }
}

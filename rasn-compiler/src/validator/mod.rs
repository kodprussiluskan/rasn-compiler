//! The `validator` module ensures that the
//! parsed data elements of the ASN1 specification
//! are valid declarations that can be further
//! processed. Among other things, the `validator`
//! assures that all dependencies of the individual
//! data elements resolve, and checks for conflicting
//! constraints and value definitions.
pub(crate) mod error;
pub(crate) mod jer;
mod linking;
mod scope;
#[cfg(test)]
mod tests;

use std::{
    cell::RefCell,
    collections::{BTreeMap, HashSet},
    rc::Rc,
};

use crate::{
    error::CompilerError,
    intermediate::{
        constraints::*,
        information_object::{ClassLink, ToplevelInformationDefinition},
        types::*,
        *,
    },
};

use self::{
    error::{LinkerError, LinkerErrorType},
    information_object::{ASN1Information, InformationObjectClassField},
    scope::{Module, Scope},
};

pub struct Validator {
    modules: BTreeMap<String, Module>,
}

impl Validator {
    pub fn new(tlds: Vec<ToplevelDefinition>) -> Validator {
        let mut modules = BTreeMap::<String, Module>::new();
        for tld in tlds {
            let header = tld.get_module_header();
            let name = header
                .as_ref()
                .map_or_else(String::new, |header| header.borrow().name.clone());
            let module = modules.entry(name).or_default();
            module.header = header;
            module.definitions.insert(tld.name().to_owned(), tld);
        }
        Self { modules }
    }

    fn link(mut self) -> Result<(Self, Vec<CompilerError>), LinkerError> {
        let mut warnings = vec![];
        // Values depend on linked types, so link them after all other definitions.
        let mut keys: Vec<_> = self
            .modules
            .iter()
            .flat_map(|(module, contents)| {
                contents.definitions.iter().map(move |(name, tld)| {
                    (
                        matches!(tld, ToplevelDefinition::Value(_)),
                        name.clone(),
                        module.clone(),
                    )
                })
            })
            .collect();
        keys.sort_unstable_by(|a, b| {
            b.0.cmp(&a.0)
                .then_with(|| a.1.cmp(&b.1))
                .then_with(|| a.2.cmp(&b.2))
        });
        let mut visited_headers = HashSet::new();
        while let Some((_, name, module)) = keys.pop() {
            let mut tld = self
                .modules
                .get_mut(&module)
                .unwrap()
                .definitions
                .remove(&name)
                .unwrap();
            let scope = Scope::new(&self.modules, &module).while_linking(&name);
            let mut warn = |error: crate::intermediate::error::GrammarError| {
                let mut error = LinkerError::from(error);
                error.contextualize(&name);
                warnings.push(error.into());
            };
            match &mut tld {
                ToplevelDefinition::Type(tld) => {
                    if tld.ty.references_class_by_name() {
                        let ty = std::mem::replace(&mut tld.ty, ASN1Type::Null);
                        tld.ty = ty.resolve_class_reference(&scope);
                    }
                    if tld.ty.contains_components_of_notation() {
                        tld.ty.link_components_of_notation(&scope);
                    }
                    if tld.ty.has_choice_selection_type() {
                        if let Err(error) = tld.ty.link_choice_selection_type(&scope) {
                            warn(error);
                        }
                    }
                }
                ToplevelDefinition::Object(tld) => {
                    if let ASN1Information::ObjectSet(set) = &mut tld.value {
                        if let Err(error) = set.resolve_object_set_references(&scope) {
                            warn(error);
                        }
                    }
                    tld.resolve_class_reference(&scope);
                    if tld.value.references_object_set_by_name() {
                        tld.value.link_object_set_reference(&scope);
                    }
                }
                _ => {}
            }
            if tld.is_parameterized() || tld.has_constraint_reference() {
                if let Err(error) = tld.link_constraint_reference(&scope) {
                    warn(error);
                }
            }
            if let Err(error) = tld.collect_supertypes(&scope) {
                warn(error);
            }
            if let Err(error) = tld.mark_recursive(&scope) {
                warn(error);
            }
            if tld.get_module_header().is_some() && visited_headers.insert(module.clone()) {
                self.fill_in_associated_type_imports(&tld, &scope);
            }
            self.modules
                .get_mut(&module)
                .unwrap()
                .definitions
                .insert(name, tld);
        }
        Ok((self, warnings))
    }

    fn fill_in_associated_type_imports(&self, tld: &ToplevelDefinition, scope: &Scope<'_>) {
        let module_header = &tld.get_module_header().unwrap();
        let mut associated_type_imports = Vec::new();
        if let ToplevelDefinition::Object(ToplevelInformationDefinition {
            class: ClassLink::ByReference(class_ref),
            ..
        }) = tld
        {
            for field in &class_ref.fields {
                self.associated_import_type_class_field(
                    field,
                    scope,
                    module_header,
                    &mut associated_type_imports,
                );
            }
        }
        for import_module in &module_header.borrow().imports {
            let source = scope.in_module(&import_module.global_module_reference.module_reference);
            for import in &import_module.types {
                if import.starts_with(|c: char| c.is_lowercase()) {
                    match source.get_qualified(Some(source.module), import) {
                        Some(ToplevelDefinition::Object(ToplevelInformationDefinition {
                            class: ClassLink::ByReference(class_ref),
                            ..
                        })) => {
                            for field in &class_ref.fields {
                                self.associated_import_type_class_field(
                                    field,
                                    &source,
                                    module_header,
                                    &mut associated_type_imports,
                                );
                            }
                        }
                        Some(ToplevelDefinition::Value(ToplevelValueDefinition {
                            associated_type: ASN1Type::ElsewhereDeclaredType(reference),
                            ..
                        })) => {
                            self.associated_import_type(
                                reference,
                                &source,
                                module_header,
                                &mut associated_type_imports,
                            );
                        }
                        _ => (),
                    }
                }
            }
        }
        let mut header = module_header.borrow_mut();
        for mut import in associated_type_imports {
            if let Some(mod_imports) = header
                .imports
                .iter_mut()
                .find(|i| i.global_module_reference == import.global_module_reference)
            {
                if !mod_imports.types.contains(&import.types[0]) {
                    mod_imports.types.push(std::mem::take(&mut import.types[0]));
                }
            } else {
                header.imports.push(import);
            }
        }
    }

    fn associated_import_type(
        &self,
        reference: &DeclarationElsewhere,
        scope: &Scope<'_>,
        module_header: &Rc<RefCell<ModuleHeader>>,
        associated_type_imports: &mut Vec<Import>,
    ) {
        if let Some(ToplevelDefinition::Type(ToplevelTypeDefinition {
            name,
            parameterization,
            module_header: Some(m_hdr),
            ..
        })) = scope.get_type(reference)
        {
            let v_type_name = format!("{}{}", name, parameterization.as_ref().map_or("", |_| "{}"));
            let v_type_mod_name = &m_hdr.borrow().name;
            if v_type_mod_name != &module_header.borrow().name
                && module_header.borrow().find_import(&v_type_name).is_none()
            {
                associated_type_imports.push(Import {
                    types: vec![v_type_name],
                    global_module_reference: GlobalModuleReference {
                        module_reference: m_hdr.borrow().name.clone(),
                        assigned_identifier: match &m_hdr.borrow().module_identifier {
                            Some(DefinitiveIdentifier::DefinitiveOID(oid))
                            | Some(DefinitiveIdentifier::DefinitiveOIDandIRI { oid, .. }) => {
                                AssignedIdentifier::ObjectIdentifierValue(oid.clone())
                            }
                            None => AssignedIdentifier::Empty,
                        },
                    },
                    with: None,
                })
            }
        }
    }

    fn associated_import_type_class_field(
        &self,
        field: &InformationObjectClassField,
        scope: &Scope<'_>,
        module_header: &Rc<RefCell<ModuleHeader>>,
        associated_type_imports: &mut Vec<Import>,
    ) {
        if let Some(ASN1Type::ElsewhereDeclaredType(DeclarationElsewhere {
            parent: Some(class_id),
            identifier,
            ..
        })) = &field.ty
        {
            if let Some(ToplevelDefinition::Class(class)) = scope.get(class_id) {
                if let Some(field) = class
                    .definition
                    .fields
                    .iter()
                    .find(|f| f.identifier.identifier() == identifier)
                {
                    self.associated_import_type_class_field(
                        field,
                        &scope.for_definition(class.module_header.as_ref()),
                        module_header,
                        associated_type_imports,
                    );
                }
            }
        } else if let Some(ASN1Type::ElsewhereDeclaredType(reference)) = &field.ty {
            self.associated_import_type(reference, scope, module_header, associated_type_imports)
        }
    }

    pub fn validate(
        mut self,
    ) -> Result<(Vec<ToplevelDefinition>, Vec<CompilerError>), CompilerError> {
        let warnings: Vec<CompilerError>;
        (self, warnings) = self.link()?;
        Ok(self
            .modules
            .into_values()
            .flat_map(|module| module.definitions.into_values())
            .fold(
                (Vec::<ToplevelDefinition>::new(), warnings),
                |(mut tlds, mut errors), tld| {
                    match tld.validate() {
                        Ok(_) => tlds.push(tld),
                        Err(e) => errors.push(e.into()),
                    }
                    (tlds, errors)
                },
            ))
    }
}

pub trait Validate {
    fn validate(&self) -> Result<(), LinkerError>;
}

impl Validate for ToplevelDefinition {
    fn validate(&self) -> Result<(), LinkerError> {
        match self {
            ToplevelDefinition::Type(t) => {
                if let Err(mut e) = t.ty.validate() {
                    e.contextualize(&t.name);
                    return Err(e);
                }
                Ok(())
            }
            ToplevelDefinition::Value(_v) => Ok(()),
            ToplevelDefinition::Class(_c) => Ok(()),
            ToplevelDefinition::Object(_o) => Ok(()),
            ToplevelDefinition::Macro(_m) => Ok(()),
        }
    }
}

impl Validate for ASN1Type {
    fn validate(&self) -> Result<(), LinkerError> {
        match self {
            ASN1Type::Integer(ref i) => i.validate(),
            ASN1Type::BitString(ref b) => b.validate(),
            ASN1Type::CharacterString(ref o) => o.validate(),
            _ => Ok(()),
        }
    }
}

impl Validate for Integer {
    fn validate(&self) -> Result<(), LinkerError> {
        for c in &self.constraints {
            c.validate()?;
        }
        Ok(())
    }
}

impl Validate for BitString {
    fn validate(&self) -> Result<(), LinkerError> {
        for c in &self.constraints {
            c.validate()?;
        }
        Ok(())
    }
}

impl Validate for CharacterString {
    fn validate(&self) -> Result<(), LinkerError> {
        for c in &self.constraints {
            c.validate()?;
        }
        Ok(())
    }
}

impl Validate for Constraint {
    fn validate(&self) -> Result<(), LinkerError> {
        if let Constraint::Subtype(c) = self {
            if let ElementOrSetOperation::Element(SubtypeElements::ValueRange {
                min,
                max,
                extensible: _,
            }) = &c.set
            {
                if let Some((ASN1Value::Integer(min), ASN1Value::Integer(max))) =
                    min.as_ref().zip(max.as_ref())
                {
                    if min > max {
                        return Err(LinkerError::new(
                            None,
                            "Mininum value exceeds maximum value!",
                            LinkerErrorType::InvalidConstraintsError,
                        ));
                    }
                }
            }
        }
        Ok(())
    }
}

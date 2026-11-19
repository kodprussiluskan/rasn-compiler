use std::{cell::RefCell, collections::BTreeMap, rc::Rc};

use crate::intermediate::{
    error::{GrammarError, GrammarErrorType},
    ASN1Type, AsnTag, DeclarationElsewhere, Exports, ModuleHeader, TaggingEnvironment,
    ToplevelDefinition, ToplevelTypeDefinition,
};

#[derive(Default)]
pub(super) struct Module {
    pub header: Option<Rc<RefCell<ModuleHeader>>>,
    pub definitions: BTreeMap<String, ToplevelDefinition>,
}

#[derive(Clone, Copy)]
pub(super) struct Scope<'a> {
    modules: &'a BTreeMap<String, Module>,
    pub module: &'a str,
    linking: Option<(&'a str, &'a str)>,
    value_module: &'a str,
    parameters: Option<&'a BTreeMap<String, ToplevelDefinition>>,
    expansion: Option<&'a Expansion<'a>>,
    resolving_actual: bool,
    linking_collection_element: bool,
    generated: Option<&'a RefCell<Vec<ToplevelTypeDefinition>>>,
}

pub(super) struct Expansion<'a> {
    template: &'a ToplevelDefinition,
    parent: Option<&'a Expansion<'a>>,
}

impl<'a> Scope<'a> {
    pub fn new(modules: &'a BTreeMap<String, Module>, module: &'a str) -> Self {
        Self {
            modules,
            module,
            linking: None,
            value_module: module,
            parameters: None,
            expansion: None,
            resolving_actual: false,
            linking_collection_element: false,
            generated: None,
        }
    }

    pub fn in_module(&self, module: &'a str) -> Self {
        Self {
            linking: self.linking,
            expansion: self.expansion,
            resolving_actual: self.resolving_actual,
            linking_collection_element: self.linking_collection_element,
            generated: self.generated,
            ..Self::new(self.modules, module)
        }
    }

    pub fn while_linking(&self, name: &'a str) -> Self {
        Self {
            linking: Some((self.module, name)),
            ..*self
        }
    }

    pub fn for_definition(&self, header: Option<&Rc<RefCell<ModuleHeader>>>) -> Self {
        let Some(header) = header else { return *self };
        let name = &header.borrow().name;
        if name == self.module {
            *self
        } else {
            self.in_module(self.modules.get_key_value(name).unwrap().0)
        }
    }

    pub fn for_value_type(&self, header: Option<&Rc<RefCell<ModuleHeader>>>) -> Self {
        Self {
            value_module: self.value_module,
            ..self.for_definition(header)
        }
    }

    pub fn for_collection_element(&self) -> Self {
        Self {
            linking_collection_element: true,
            ..*self
        }
    }

    pub fn linking_collection_element(&self) -> bool {
        self.linking_collection_element
    }

    pub fn value_scope(&self) -> Self {
        if self.module == self.value_module {
            *self
        } else {
            self.in_module(self.value_module)
        }
    }

    pub fn with_parameters(&self, parameters: &'a BTreeMap<String, ToplevelDefinition>) -> Self {
        Self {
            parameters: Some(parameters),
            ..*self
        }
    }

    pub fn parameter(&self, name: &str) -> Option<&'a ToplevelDefinition> {
        self.parameters?.get(name)
    }

    pub fn expansion_for(&self, template: &'a ToplevelDefinition) -> Option<Expansion<'a>> {
        let mut ancestor = self.expansion;
        while let Some(expansion) = ancestor {
            if std::ptr::eq(expansion.template, template) {
                return None;
            }
            ancestor = expansion.parent;
        }
        Some(Expansion {
            template,
            parent: self.expansion,
        })
    }

    pub fn with_expansion(&self, expansion: &'a Expansion<'a>) -> Self {
        Self {
            expansion: Some(expansion),
            ..*self
        }
    }

    pub fn for_actual(&self) -> Self {
        Self {
            resolving_actual: true,
            ..*self
        }
    }

    pub fn in_expansion(&self) -> bool {
        self.resolving_actual || self.expansion.is_some()
    }

    pub fn with_generated_types(
        &self,
        generated: &'a RefCell<Vec<ToplevelTypeDefinition>>,
    ) -> Self {
        Self {
            generated: Some(generated),
            ..*self
        }
    }

    fn module_tagging_environment(&self, name: &str) -> TaggingEnvironment {
        self.modules
            .get(name)
            .and_then(|module| module.header.as_ref())
            .map_or(TaggingEnvironment::default(), |header| {
                header.borrow().tagging_environment
            })
    }

    pub fn tagging_environment(&self) -> TaggingEnvironment {
        self.module_tagging_environment(self.module)
    }

    pub fn output_tagging_environment(&self) -> TaggingEnvironment {
        self.module_tagging_environment(self.linking.map_or(self.module, |(module, _)| module))
    }

    pub fn materialize_type(
        &self,
        ty: ASN1Type,
        tag: Option<AsnTag>,
        environment: Option<TaggingEnvironment>,
    ) -> Result<ASN1Type, GrammarError> {
        let (Some(generated), Some((module, declaration))) = (self.generated, self.linking) else {
            return Err(GrammarError::new(
                "No declaration owns the tagged specialization",
                GrammarErrorType::LinkerError,
            ));
        };
        let mut generated = generated.borrow_mut();
        let mut index = generated.len() + 1;
        let name = loop {
            let name = format!("{declaration}-Specialization-{index}");
            if self.local(module, &name).is_none()
                && !generated.iter().any(|definition| definition.name == name)
            {
                break name;
            }
            index += 1;
        };
        let mut definition = ToplevelTypeDefinition::from((name.as_str(), ty));
        definition.tag = tag;
        definition.module_header = self
            .modules
            .get(module)
            .and_then(|module| module.header.clone());
        if let (Some(environment), Some(header)) = (environment, &definition.module_header) {
            let mut header = header.borrow().clone();
            header.tagging_environment = environment;
            definition.module_header = Some(Rc::new(RefCell::new(header)));
        }
        generated.push(definition);
        Ok(ASN1Type::ElsewhereDeclaredType(DeclarationElsewhere {
            parent: None,
            module: Some(module.to_owned()),
            identifier: name,
            constraints: vec![],
        }))
    }

    fn local(&self, module: &str, name: &str) -> Option<&'a ToplevelDefinition> {
        self.modules.get(module)?.definitions.get(name)
    }

    fn is_local_name(&self, name: &str) -> bool {
        self.linking == Some((self.module, name))
            || self.local(self.module, name).is_some()
            || self
                .parameters
                .is_some_and(|parameters| parameters.contains_key(name))
    }

    pub fn reference_module(&self, qualifier: Option<&str>, name: &str) -> Option<&'a str> {
        if let Some(module) = qualifier {
            return self
                .modules
                .get_key_value(module)
                .map(|(name, _)| name.as_str());
        }
        if self.is_local_name(name) {
            return Some(self.module);
        }
        let header = self.modules.get(self.module)?.header.as_ref();
        let Some(header) = header else {
            return Some(self.module);
        };
        let header = header.borrow();
        let mut imports = header.imported_modules(name);
        let Some(module) = imports.next() else {
            return Some(self.module);
        };
        if imports.any(|other| other != module) {
            return None;
        }
        self.modules
            .get_key_value(module)
            .map(|(name, _)| name.as_str())
    }

    pub fn has_ambiguous_type_reference(&self, reference: &DeclarationElsewhere) -> bool {
        reference.module.is_none()
            && !self.is_local_name(&reference.identifier)
            && self
                .modules
                .get(self.module)
                .and_then(|module| module.header.as_ref())
                .is_some_and(|header| {
                    header
                        .borrow()
                        .has_conflicting_import(&reference.identifier)
                })
    }

    pub fn get_qualified(
        &self,
        qualifier: Option<&str>,
        name: &str,
    ) -> Option<&'a ToplevelDefinition> {
        if qualifier.is_none() {
            if let Some(parameter) = self.parameters.and_then(|parameters| parameters.get(name)) {
                return Some(parameter);
            }
        }
        self.local(self.reference_module(qualifier, name)?, name)
    }

    pub fn get(&self, name: &str) -> Option<&'a ToplevelDefinition> {
        self.get_qualified(None, name)
    }

    pub fn get_type(&self, reference: &DeclarationElsewhere) -> Option<&'a ToplevelDefinition> {
        self.get_qualified(reference.module.as_deref(), &reference.identifier)
    }

    /// Resolves a parameterized assignment through the ASN.1 module boundary.
    pub fn get_template(
        &self,
        reference: &DeclarationElsewhere,
    ) -> Result<&'a ToplevelDefinition, GrammarError> {
        let name = reference.identifier.as_str();
        let error = |message: String| GrammarError::new(&message, GrammarErrorType::LinkerError);
        if reference.module.is_none() {
            if let Some(parameter) = self.parameters.and_then(|parameters| parameters.get(name)) {
                return Ok(parameter);
            }
            if let Some(local) = self.local(self.module, name) {
                return Ok(local);
            }
        }
        let header = self.modules.get(self.module).and_then(|module| module.header.as_ref())
            .ok_or_else(|| error(format!("Missing import of parameterized type {name}")))?;
        let header = header.borrow();
        let imports: Vec<_> = header.imports.iter().filter(|import| {
            import.types.iter().any(|symbol| symbol == name || symbol.strip_suffix("{}") == Some(name))
                && reference.module.as_ref().is_none_or(|module| {
                    module == &import.global_module_reference.module_reference
                })
        }).collect();
        if imports.len() != 1 {
            let reason = if imports.is_empty() { "missing import" } else { "ambiguous import" };
            return Err(error(format!("{reason} for parameterized type {name}")));
        }
        let import = imports[0];
        if import.with.is_some() {
            return Err(error(format!("Unsupported module selection for parameterized type {name}")));
        }
        self.exported_template(
            &import.global_module_reference.module_reference,
            name,
            &mut Vec::new(),
        )
    }

    fn exported_template(
        &self,
        module: &str,
        name: &str,
        visited: &mut Vec<String>,
    ) -> Result<&'a ToplevelDefinition, GrammarError> {
        let error = |message: String| GrammarError::new(&message, GrammarErrorType::LinkerError);
        if visited.iter().any(|seen| seen == module) {
            return Err(error(format!("Re-export cycle for parameterized type {module}.{name}")));
        }
        visited.push(module.to_owned());
        let contents = self.modules.get(module)
            .ok_or_else(|| error(format!("Missing module {module} for parameterized type {name}")))?;
        let header = contents.header.as_ref()
            .ok_or_else(|| error(format!("Missing module header for {module}")))?;
        let header = header.borrow();
        if matches!(&header.exports, Some(Exports::Identifier(names)) if !names.iter().any(|symbol| symbol == name)) {
            return Err(error(format!("Module {module} does not export parameterized type {name}")));
        }
        let imports: Vec<_> = header.imports.iter().filter(|import| {
            import.types.iter().any(|symbol| symbol == name || symbol.strip_suffix("{}") == Some(name))
        }).collect();
        if let Some(definition) = contents.definitions.get(name) {
            if !imports.is_empty() {
                return Err(error(format!("ambiguous re-export {module}.{name}")));
            }
            return Ok(definition);
        }
        if imports.len() != 1 {
            return Err(error(format!("ambiguous or missing re-export {module}.{name}")));
        }
        if imports[0].with.is_some() {
            return Err(error(format!("Unsupported module selection for parameterized type {module}.{name}")));
        }
        self.exported_template(&imports[0].global_module_reference.module_reference, name, visited)
    }

    pub fn visible_definitions(&self) -> impl Iterator<Item = &'a ToplevelDefinition> + '_ {
        self.parameters
            .into_iter()
            .flat_map(|parameters| parameters.values())
            .chain(self.modules.iter().flat_map(move |(module, contents)| {
                contents.definitions.iter().filter_map(move |(name, tld)| {
                    self.reference_module(None, name)
                        .is_some_and(|visible_module| visible_module == module)
                        .then_some(tld)
                })
            }))
    }
}

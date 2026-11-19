use std::{cell::RefCell, collections::BTreeMap, rc::Rc};

use crate::intermediate::{
    error::{GrammarError, GrammarErrorType},
    DeclarationElsewhere, Exports, ModuleHeader, ToplevelDefinition,
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
}

impl<'a> Scope<'a> {
    pub fn new(modules: &'a BTreeMap<String, Module>, module: &'a str) -> Self {
        Self {
            modules,
            module,
            linking: None,
            value_module: module,
            parameters: None,
        }
    }

    pub fn in_module(&self, module: &'a str) -> Self {
        Self {
            linking: self.linking,
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

use super::Scope;

use super::{
    ASN1Type, DeclarationElsewhere, GrammarError, ToplevelDefinition, ToplevelTypeDefinition,
};

impl DeclarationElsewhere {
    pub(super) fn check_ambiguity(&self, scope: &Scope<'_>) -> Result<(), GrammarError> {
        if scope.has_ambiguous_type_reference(self) {
            return Err(GrammarError::new(
                &format!("Ambiguous type reference '{}' in module '{}'. Qualify it with a module name.", self.identifier, scope.module),
                super::GrammarErrorType::LinkerError,
            ));
        }
        Ok(())
    }

    pub(in crate::validator) fn root<'a>(
        &self,
        tlds: &Scope<'a>,
    ) -> Result<&'a ASN1Type, GrammarError> {
        match tlds.get_type(self).ok_or_else(|| GrammarError::new(
            &format!("Failed to resolve reference of ElsewhereDefined: {}", self.identifier),
            super::GrammarErrorType::LinkerError
        ))? {
            ToplevelDefinition::Type(ToplevelTypeDefinition { ty: ASN1Type::ElsewhereDeclaredType(e), module_header, .. }) => e.root(&tlds.for_definition(module_header.as_ref())),
            ToplevelDefinition::Type(ToplevelTypeDefinition { ty, .. }) => Ok(ty),
            ToplevelDefinition::Class(_) => Err(GrammarError::todo()),
            ToplevelDefinition::Object(_) => Err(GrammarError::todo()),
            _ => Err(GrammarError::new(
                &format!(
                    "Unexpectedly found a value definition resolving reference of ElsewhereDefined: {}",
                    self.identifier
                ),
                super::GrammarErrorType::LinkerError
            ))
        }
    }
}

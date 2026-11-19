use std::collections::{HashMap, HashSet};

use super::error::{LinkerError, LinkerErrorType};
use crate::intermediate::{
    ASN1Type, ASN1Value, CharacterStringType, DeclarationElsewhere, ModuleHeader, ToplevelDefinition,
    ToplevelTypeDefinition,
    jer::{Assignment, Category, Instruction, Rename, Representation, Resolved, Target},
    types::Optionality,
};

pub(crate) fn resolve(definitions: &mut Vec<ToplevelDefinition>) -> Result<(), LinkerError> {
    if !definitions.iter().any(|definition| {
        if let ToplevelDefinition::Type(definition) = definition {
            definition
                .module_header
                .as_ref()
                .is_some_and(|header| !header.borrow().jer.is_empty())
                || has_prefix(&definition.ty)
        } else {
            false
        }
    }) {
        return Ok(());
    }

    let mut types = HashMap::new();
    let mut headers = HashMap::new();
    let mut values = HashMap::new();
    for definition in &*definitions {
        if let ToplevelDefinition::Value(value) = definition {
            if let Some(header) = &value.module_header {
                let module = header.borrow().name.clone();
                values.insert((module, value.name.clone()), value.value.clone());
            }
        }
    }
    for definition in &mut *definitions {
        if let ToplevelDefinition::Type(definition) = definition {
            let header = definition
                .module_header
                .as_ref()
                .expect("module assigned before JER resolution")
                .borrow();
            for assignment in &header.jer {
                for target in &assignment.targets {
                    if let Target::Imports(module) = target {
                        if !header
                            .imports
                            .iter()
                            .any(|import| import.global_module_reference.module_reference == *module)
                        {
                            return Err(LinkerError::new(
                                Some(definition.name.clone()),
                                &format!("JER ALL IMPORTS FROM refers to unimported module {module}"),
                                LinkerErrorType::MissingDependency,
                            ));
                        }
                    }
                }
            }
            attach(&mut definition.ty, &header, Some(&definition.name));
            headers.entry(header.name.clone()).or_insert_with(|| header.clone());
            types.insert((header.name.clone(), definition.name.clone()), definition.ty.clone());
        }
    }
    let mut synthesized = Vec::new();
    for definition in &mut *definitions {
        if let ToplevelDefinition::Type(definition) = definition {
            let header = definition
                .module_header
                .as_ref()
                .expect("module assigned before JER resolution")
                .clone();
            let mut lower = Lower {
                header: &header.borrow(),
                owner: &definition.name,
                types: &types,
                headers: &headers,
                values: &values,
                synthesized: &mut synthesized,
            };
            let (resolved, _) = lower.occurrence(&mut definition.ty)?;
            definition.jer = resolved;
        }
    }
    definitions.extend(synthesized);
    Ok(())
}

fn has_prefix(ty: &ASN1Type) -> bool {
    match ty {
        ASN1Type::Jer(_, _) => true,
        ASN1Type::Set(sequence) | ASN1Type::Sequence(sequence) => {
            sequence.members.iter().any(|member| has_prefix(&member.ty))
        }
        ASN1Type::Choice(choice) => choice.options.iter().any(|option| has_prefix(&option.ty)),
        ASN1Type::SetOf(collection) | ASN1Type::SequenceOf(collection) => has_prefix(&collection.element_type),
        _ => false,
    }
}

fn attach(ty: &mut ASN1Type, header: &ModuleHeader, root: Option<&str>) {
    let (explicit, inner) = match ty {
        ASN1Type::Jer(explicit, inner) => (explicit.as_slice(), inner.as_mut()),
        _ => (&[][..], &mut *ty),
    };
    let mut matches = |target: &Target| match target {
        Target::All => root.is_some(),
        Target::BuiltIn(kind) => category(inner) == Some(*kind),
        Target::Imports(module) => {
            if let ASN1Type::ElsewhereDeclaredType(reference) = inner {
                header.imports.iter().any(|import| {
                    import.global_module_reference.module_reference == *module
                        && import.types.contains(&reference.identifier)
                        && reference.module.as_ref().map_or(true, |qualifier| qualifier == module)
                })
            } else {
                false
            }
        }
    };
    let mut instructions = Vec::new();
    for imports in [true, false] {
        for Assignment { instruction, targets } in &header.jer {
            if targets
                .iter()
                .any(|target| matches(target) && imports == matches!(target, Target::Imports(_)))
            {
                instructions.push((true, instruction.clone()));
            }
        }
    }
    match inner {
        ASN1Type::Sequence(components) | ASN1Type::Set(components) => {
            for member in &mut components.members {
                attach(&mut member.ty, header, None);
            }
        }
        ASN1Type::Choice(choice) => {
            for option in &mut choice.options {
                attach(&mut option.ty, header, None);
            }
        }
        ASN1Type::SequenceOf(collection) | ASN1Type::SetOf(collection) => {
            attach(&mut collection.element_type, header, None);
        }
        _ => {}
    }
    if !instructions.is_empty() {
        instructions.reverse();
        let mut combined = explicit.to_vec();
        combined.extend(instructions);
        let plain = std::mem::replace(inner, ASN1Type::Null);
        *ty = ASN1Type::Jer(combined, Box::new(plain));
    }
}

fn category(ty: &ASN1Type) -> Option<Category> {
    Some(match ty {
        ASN1Type::Sequence(_) => Category::Array,
        ASN1Type::OctetString(_) => Category::Base64,
        ASN1Type::SetOf(_) => Category::Object,
        ASN1Type::Enumerated(_) => Category::Text,
        ASN1Type::Choice(_) => Category::Unwrapped,
        _ => return None,
    })
}

fn reference_module<'a>(reference: &'a DeclarationElsewhere, header: &'a ModuleHeader) -> &'a str {
    reference
        .module
        .as_deref()
        .or_else(|| {
            header
                .imports
                .iter()
                .find(|import| import.types.contains(&reference.identifier))
                .map(|import| import.global_module_reference.module_reference.as_str())
        })
        .unwrap_or(&header.name)
}

type Types = HashMap<(String, String), ASN1Type>;

struct Lower<'a> {
    header: &'a ModuleHeader,
    owner: &'a str,
    types: &'a Types,
    headers: &'a HashMap<String, ModuleHeader>,
    values: &'a HashMap<(String, String), ASN1Value>,
    synthesized: &'a mut Vec<ToplevelDefinition>,
}

impl Lower<'_> {
    fn error(&self, details: &str) -> LinkerError {
        LinkerError::new(
            Some(self.owner.into()),
            details,
            LinkerErrorType::InvalidConstraintsError,
        )
    }

    fn replacement(&self, rename: &Rename, original: &str, module: &str) -> Result<String, LinkerError> {
        let Rename::Reference {
            module: qualifier,
            identifier,
        } = rename
        else {
            return Ok(rename.apply(original).expect("literal or case conversion"));
        };
        let mut qualifier = qualifier.as_deref();
        let mut identifier = identifier.as_str();
        let mut scope = module;
        let mut visited = HashSet::new();
        loop {
            let imported = self.headers.get(scope).and_then(|header| {
                header
                    .imports
                    .iter()
                    .find(|import| import.types.iter().any(|name| name == identifier))
                    .map(|import| import.global_module_reference.module_reference.as_str())
            });
            let target = qualifier.or(imported).unwrap_or(scope);
            if target != scope && imported != Some(target) {
                return Err(self.error("JER string value reference must be local or imported"));
            }
            if !visited.insert((target, identifier)) {
                return Err(self.error("cyclic JER string value reference"));
            }
            match self.values.get(&(target.to_owned(), identifier.to_owned())) {
                Some(ASN1Value::String(text)) => return Ok(text.clone()),
                Some(ASN1Value::ElsewhereDeclaredValue {
                    module,
                    identifier: next,
                    parent: None,
                }) => {
                    qualifier = module.as_deref();
                    identifier = next;
                    scope = target;
                }
                _ => return Err(self.error("JER name or text requires a character string value")),
            }
        }
    }

    fn type_and_prefixes<'a>(
        &'a self,
        mut ty: &'a ASN1Type,
        mut module: &'a str,
    ) -> (&'a ASN1Type, &'a str, Vec<&'a Instruction>) {
        let mut instructions = Vec::new();
        let mut seen = HashSet::new();
        loop {
            match ty {
                ASN1Type::Jer(prefixes, inner) => {
                    instructions.extend(prefixes.iter().map(|(_, instruction)| instruction));
                    ty = inner;
                }
                ASN1Type::ElsewhereDeclaredType(reference) => {
                    module = self
                        .headers
                        .get(module)
                        .map_or(module, |header| reference_module(reference, header));
                    if !seen.insert((module, reference.identifier.as_str())) {
                        break;
                    }
                    let Some(referenced) = self.types.get(&(module.into(), reference.identifier.clone())) else {
                        break;
                    };
                    ty = referenced;
                }
                _ => break,
            }
        }
        (ty, module, instructions)
    }

    fn occurrence(&mut self, ty: &mut ASN1Type) -> Result<(Resolved, Option<Rename>), LinkerError> {
        let mut resolved = Resolved::default();
        let mut name = None;
        if let ASN1Type::Jer(_, _) = ty {
            let ASN1Type::Jer(instructions, inner) = std::mem::replace(ty, ASN1Type::Null) else {
                unreachable!()
            };
            *ty = *inner;
            let mut final_instructions = Vec::new();
            for (qualified, instruction) in instructions.into_iter().rev() {
                if !qualified
                    && !self
                        .header
                        .encoding_reference_default
                        .as_ref()
                        .is_some_and(|default| default.0 == "JER")
                {
                    return Err(self.error("a bare JER prefix requires JER INSTRUCTIONS"));
                }
                let kind = instruction_category(&instruction);
                final_instructions.retain(|existing| instruction_category(existing) != kind);
                final_instructions.push(instruction);
            }
            let (base, module, _) = self.type_and_prefixes(ty, &self.header.name);
            for instruction in final_instructions {
                let kind = instruction_category(&instruction);
                match instruction {
                    Instruction::Disable(Category::Name) => {}
                    Instruction::Name(rename) => name = Some(rename),
                    Instruction::Disable(Category::Text) if category(base) == Some(kind) => {
                        resolved.text = Some(Vec::new())
                    }
                    Instruction::Disable(_) if category(base) == Some(kind) => {
                        resolved.representation = Some(Representation::Default)
                    }
                    Instruction::Disable(_) => {}
                    Instruction::Text(changes) => {
                        let ASN1Type::Enumerated(enumeration) = base else {
                            return Err(self.error("JER TEXT requires ENUMERATED"));
                        };
                        let mut replacements = HashMap::new();
                        for (item, rename) in changes {
                            if item.is_none() && matches!(rename, Rename::Literal(_) | Rename::Reference { .. }) {
                                return Err(self.error("JER TEXT ALL requires a case conversion"));
                            }
                            if item
                                .as_ref()
                                .is_some_and(|item| !enumeration.members.iter().any(|member| member.name == *item))
                            {
                                return Err(self.error("unknown JER TEXT item"));
                            }
                            if replacements.insert(item, rename).is_some() {
                                return Err(self.error("duplicate JER TEXT item"));
                            }
                        }
                        let mut labels = HashSet::new();
                        let mut text = Vec::new();
                        for member in &enumeration.members {
                            let replacement = replacements
                                .get(&Some(member.name.clone()))
                                .or_else(|| replacements.get(&None));
                            let label = replacement.map_or_else(
                                || Ok(member.name.clone()),
                                |rename| self.replacement(rename, &member.name, &self.header.name),
                            )?;
                            if !labels.insert(label.clone()) {
                                return Err(self.error("duplicate final JER TEXT value"));
                            }
                            if replacement.is_some() {
                                text.push((member.name.clone(), label));
                            }
                        }
                        resolved.text = Some(text);
                    }
                    Instruction::Enable(kind) => {
                        if category(base) != Some(kind) {
                            return Err(self.error(&format!("JER {kind:?} cannot be applied to {}", base.as_str())));
                        }
                        resolved.representation = Some(match kind {
                            Category::Array => {
                                let ASN1Type::Sequence(sequence) = base else {
                                    unreachable!()
                                };
                                self.validate_array(sequence, module)?;
                                Representation::Array
                            }
                            Category::Base64 => Representation::Base64,
                            Category::Object => {
                                let ASN1Type::SetOf(collection) = base else {
                                    unreachable!()
                                };
                                self.validate_object(&collection.element_type, module)?;
                                Representation::Object
                            }
                            Category::Unwrapped => {
                                let ASN1Type::Choice(choice) = base else { unreachable!() };
                                self.validate_unwrapped(choice, module)?;
                                Representation::Unwrapped
                            }
                            Category::Name | Category::Text => unreachable!(),
                        });
                    }
                }
            }
        }
        let mut names = HashSet::new();
        match ty {
            ASN1Type::Sequence(components) | ASN1Type::Set(components) => {
                for member in &mut components.members {
                    let rename = self.nested(&mut member.ty)?;
                    self.component_name(&member.name, rename, &mut names, &mut resolved)?;
                }
            }
            ASN1Type::Choice(choice) => {
                for option in &mut choice.options {
                    let rename = self.nested(&mut option.ty)?;
                    self.component_name(&option.name, rename, &mut names, &mut resolved)?;
                }
            }
            ASN1Type::SequenceOf(collection) | ASN1Type::SetOf(collection) => {
                self.nested(&mut collection.element_type)?;
            }
            _ => {}
        }
        Ok((resolved, name))
    }

    fn component_name(
        &self,
        original: &str,
        rename: Option<Rename>,
        names: &mut HashSet<String>,
        resolved: &mut Resolved,
    ) -> Result<(), LinkerError> {
        let name = rename.as_ref().map_or_else(
            || Ok(original.into()),
            |rename| self.replacement(rename, original, &self.header.name),
        )?;
        if !names.insert(name.clone()) {
            return Err(self.error("duplicate final JER component name"));
        }
        if rename.is_some() {
            resolved.names.push((original.into(), name));
        }
        Ok(())
    }

    fn nested(&mut self, ty: &mut ASN1Type) -> Result<Option<Rename>, LinkerError> {
        let (jer, name) = self.occurrence(ty)?;
        if !jer.is_empty() {
            let mut index = self.synthesized.len();
            let identifier = loop {
                let name = format!("{}-Jer-{index}", self.owner);
                if !self.types.keys().any(|(_, existing)| *existing == name)
                    && !self.synthesized.iter().any(|definition| definition.name() == &name)
                {
                    break name;
                }
                index += 1;
            };
            let inner = std::mem::replace(
                ty,
                ASN1Type::ElsewhereDeclaredType(DeclarationElsewhere {
                    parent: None,
                    module: None,
                    identifier: identifier.clone(),
                    constraints: Vec::new(),
                }),
            );
            let mut definition = ToplevelTypeDefinition::from((identifier.as_str(), inner));
            definition.module_header = Some(std::rc::Rc::new(std::cell::RefCell::new(self.header.clone())));
            definition.jer = jer;
            self.synthesized.push(ToplevelDefinition::Type(definition));
        }
        Ok(name)
    }

    fn validate_array(&self, seq: &crate::intermediate::types::SequenceOrSet, module: &str) -> Result<(), LinkerError> {
        for (index, member) in seq.members.iter().enumerate() {
            let extension = seq.extensible.is_some_and(|root| index >= root);
            if extension
                && member
                    .name
                    .starts_with(crate::common::INTERNAL_EXTENSION_GROUP_NAME_PREFIX)
            {
                let (base, module, _) = self.type_and_prefixes(&member.ty, module);
                if let ASN1Type::Sequence(group) = base {
                    for member in &group.members {
                        self.validate_array_slot(&member.ty, module)?;
                    }
                    continue;
                }
            }
            if extension || !matches!(member.optionality, Optionality::Required) {
                self.validate_array_slot(&member.ty, module)?;
            }
        }
        Ok(())
    }

    fn validate_array_slot(&self, ty: &ASN1Type, module: &str) -> Result<(), LinkerError> {
        let shape = self.shape(ty, module, &mut HashSet::new())?;
        if shape.kinds & Shape::NULL != 0 || shape.unknown {
            return Err(self.error(
                "JER ARRAY optional, default and extension components must have unambiguous non-null encodings",
            ));
        }
        Ok(())
    }

    fn validate_unwrapped(&self, choice: &crate::intermediate::types::Choice, module: &str) -> Result<(), LinkerError> {
        let mut alternatives = Vec::new();
        for option in &choice.options {
            let shape = self.shape(&option.ty, module, &mut HashSet::new())?;
            if shape.unknown || alternatives.iter().any(|other| shape.overlaps(other)) {
                return Err(self.error("JER UNWRAPPED choice alternatives have overlapping JSON encodings"));
            }
            alternatives.push(shape);
        }
        Ok(())
    }

    fn shape(
        &self,
        ty: &ASN1Type,
        module: &str,
        visiting: &mut HashSet<*const ASN1Type>,
    ) -> Result<Shape, LinkerError> {
        let (base, module, instructions) = self.type_and_prefixes(ty, module);
        let enabled = |category| {
            matches!(
                instructions
                    .iter()
                    .find(|instruction| instruction_category(instruction) == category),
                Some(Instruction::Enable(_))
            )
        };
        Ok(match base {
            ASN1Type::Null => Shape::new(Shape::NULL),
            ASN1Type::Boolean(_) => Shape::new(Shape::BOOL),
            ASN1Type::Integer(_) => Shape::new(Shape::NUMBER),
            ASN1Type::Real(_) => Shape::new(Shape::NUMBER | Shape::STRING | Shape::OBJECT),
            ASN1Type::OctetString(_) if !enabled(Category::Base64) && has_contents(base) => {
                Shape::new(Shape::STRING | Shape::OBJECT)
            }
            ASN1Type::BitString(bits) => {
                let fixed = crate::intermediate::encoding_rules::per_visible::per_visible_range_constraints(
                    false,
                    &bits.constraints,
                )
                .is_ok_and(|range| {
                    !range.is_extensible()
                        && range.min::<i128>().is_some()
                        && range.min::<i128>() == range.max::<i128>()
                });
                Shape::new(if !fixed || has_contents(base) {
                    Shape::OBJECT
                } else {
                    Shape::STRING
                })
            }
            ASN1Type::Sequence(_) if enabled(Category::Array) => Shape::new(Shape::ARRAY),
            ASN1Type::SequenceOf(_) => Shape::new(Shape::ARRAY),
            ASN1Type::SetOf(_) if !enabled(Category::Object) => Shape::new(Shape::ARRAY),
            ASN1Type::Sequence(components) | ASN1Type::Set(components) => {
                let mut shape = Shape::new(Shape::OBJECT);
                if components.extensible.is_none()
                    && !self.headers.get(module).is_some_and(|header| {
                        matches!(
                            header.extensibility_environment,
                            crate::intermediate::ExtensibilityEnvironment::Implied
                        )
                    })
                {
                    shape.members = Some(
                        components
                            .members
                            .iter()
                            .map(|member| {
                                let mut name = member.name.clone();
                                if let ASN1Type::Jer(instructions, _) = &member.ty {
                                    if let Some(Instruction::Name(rename)) = instructions
                                        .iter()
                                        .map(|(_, item)| item)
                                        .find(|item| instruction_category(item) == Category::Name)
                                    {
                                        name = self.replacement(rename, &name, module)?;
                                    }
                                }
                                Ok((name, matches!(member.optionality, Optionality::Required)))
                            })
                            .collect::<Result<Vec<_>, LinkerError>>()?,
                    );
                }
                shape
            }
            ASN1Type::Choice(choice) if enabled(Category::Unwrapped) => {
                if choice.extensible.is_some() || !visiting.insert(base) {
                    return Ok(Shape::unknown());
                }
                let mut shape = Shape::new(0);
                for option in &choice.options {
                    let member = self.shape(&option.ty, module, visiting)?;
                    shape.kinds |= member.kinds;
                    shape.unknown |= member.unknown;
                }
                visiting.remove(&(base as *const _));
                shape
            }
            ASN1Type::SetOf(_) | ASN1Type::Choice(_) | ASN1Type::EmbeddedPdv | ASN1Type::External => {
                Shape::new(Shape::OBJECT)
            }
            ASN1Type::Any
            | ASN1Type::ObjectClassField(_)
            | ASN1Type::ElsewhereDeclaredType(_)
            | ASN1Type::ChoiceSelectionType(_) => Shape::unknown(),
            _ => Shape::new(Shape::STRING),
        })
    }

    fn validate_object(&self, element: &ASN1Type, module: &str) -> Result<(), LinkerError> {
        let (element, module, _) = self.type_and_prefixes(element, module);
        let ASN1Type::Sequence(pair) = element else {
            return Err(self.error("JER OBJECT requires a SEQUENCE element"));
        };
        if pair.extensible.is_some()
            || self.headers.get(module).is_some_and(|header| {
                matches!(
                    header.extensibility_environment,
                    crate::intermediate::ExtensibilityEnvironment::Implied
                )
            })
            || pair.members.len() != 2
            || pair
                .members
                .iter()
                .any(|member| !matches!(member.optionality, Optionality::Required))
        {
            return Err(self.error("JER OBJECT requires a non-extensible SEQUENCE with two required components"));
        }
        if !matches!(
            self.type_and_prefixes(&pair.members[0].ty, module).0,
            ASN1Type::Enumerated(_)
                | ASN1Type::CharacterString(crate::intermediate::types::CharacterString {
                    ty: CharacterStringType::IA5String
                        | CharacterStringType::VisibleString
                        | CharacterStringType::NumericString
                        | CharacterStringType::PrintableString
                        | CharacterStringType::BMPString
                        | CharacterStringType::UniversalString
                        | CharacterStringType::UTF8String,
                    ..
                })
        ) {
            return Err(self.error("JER OBJECT requires a supported string or ENUMERATED key"));
        }
        Ok(())
    }
}

fn instruction_category(instruction: &Instruction) -> Category {
    match instruction {
        Instruction::Enable(kind) | Instruction::Disable(kind) => *kind,
        Instruction::Name(_) => Category::Name,
        Instruction::Text(_) => Category::Text,
    }
}

fn has_contents(ty: &ASN1Type) -> bool {
    use crate::intermediate::constraints::{Constraint, ContentConstraint};
    ty.constraints()
        .iter()
        .any(|constraint| matches!(constraint, Constraint::Content(ContentConstraint::Containing(_))))
}

struct Shape {
    kinds: u8,
    members: Option<Vec<(String, bool)>>,
    unknown: bool,
}

impl Shape {
    const NULL: u8 = 1;
    const BOOL: u8 = 2;
    const NUMBER: u8 = 4;
    const STRING: u8 = 8;
    const ARRAY: u8 = 16;
    const OBJECT: u8 = 32;

    fn new(kinds: u8) -> Self {
        Self {
            kinds,
            members: None,
            unknown: false,
        }
    }

    fn unknown() -> Self {
        Self {
            unknown: true,
            ..Self::new(0)
        }
    }

    fn overlaps(&self, other: &Self) -> bool {
        let common = self.kinds & other.kinds;
        if common == Self::OBJECT {
            if let (Some(left), Some(right)) = (&self.members, &other.members) {
                let unique_required = |a: &[(String, bool)], b: &[(String, bool)]| {
                    a.iter()
                        .any(|(name, required)| *required && !b.iter().any(|(other, _)| name == other))
                };
                return !unique_required(left, right) && !unique_required(right, left);
            }
        }
        common != 0
    }
}

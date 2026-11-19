use std::collections::{BTreeMap, BTreeSet};

use proc_macro2::{Literal, TokenStream};
use quote::quote;

use crate::{
    generator::error::{GeneratorError, GeneratorErrorType},
    intermediate::{
        constraints::{Constraint, ElementOrSetOperation, SetOperator, SubtypeElements},
        encoding_rules::per_visible::per_visible_range_constraints,
        types::SequenceOrSetOf,
        ASN1Type, ASN1Value, ModuleHeader,
    },
};

use super::{
    copy::{resolve, TypeDeclarations},
    Rasn,
};

type TypeId = (String, String);
type Intervals = Vec<(i128, i128)>;

#[derive(Debug, Default)]
pub(super) struct CollectionLayout {
    cyclic: Vec<(TypeId, SequenceOrSetOf)>,
}

impl Rasn {
    pub(super) fn collection_array_length(
        &self,
        collection: &SequenceOrSetOf,
    ) -> Result<Option<u32>, GeneratorError> {
        let owner = self.collection_owner.borrow();
        self.collection_array_length_for(
            collection,
            self.copy_module.as_ref().and_then(|header| {
                owner
                    .as_ref()
                    .map(|name| (header.name.as_str(), name.as_str()))
            }),
        )
    }

    pub(super) fn collection_array_length_for(
        &self,
        collection: &SequenceOrSetOf,
        owner: Option<(&str, &str)>,
    ) -> Result<Option<u32>, GeneratorError> {
        if !self.fixed_size_arrays {
            return Ok(None);
        }
        let Some(layout) = &self.collection_layout else {
            return Ok(None);
        };
        if layout.cyclic.iter().any(|((module, name), list)| {
            Some((module.as_str(), name.as_str())) == owner && list == collection
        }) {
            return Ok(None);
        }
        fixed_count(&collection.constraints)
    }

    pub(super) fn collection_type(
        &self,
        collection: &SequenceOrSetOf,
        is_set: bool,
        element: TokenStream,
    ) -> Result<TokenStream, GeneratorError> {
        if is_set {
            return Ok(quote!(SetOf<#element>));
        }
        Ok(match self.collection_array_length(collection)? {
            Some(count) => {
                let count = Literal::u32_unsuffixed(count);
                quote!([#element; #count])
            }
            None => quote!(SequenceOf<#element>),
        })
    }

    pub(super) fn collect_collection_layout(
        &self,
        declarations: &TypeDeclarations<'_>,
    ) -> Result<CollectionLayout, GeneratorError> {
        let mut result = CollectionLayout::default();
        if !self.fixed_size_arrays {
            return Ok(result);
        }
        let mut graph = BTreeMap::new();
        for (id, declaration) in declarations {
            if declaration.parameterization.is_some() {
                continue;
            }
            let header = declaration.module_header.as_ref().unwrap().borrow();
            let mut edges = BTreeSet::new();
            inline_references(&declaration.ty, &header, declarations, &mut edges)?;
            graph.insert(id.clone(), edges);
        }
        for (id, declaration) in declarations {
            if declaration.parameterization.is_some() {
                continue;
            }
            let header = declaration.module_header.as_ref().unwrap().borrow();
            let mut candidates = Vec::new();
            collection_candidates(&declaration.ty, &mut candidates);
            for collection in candidates {
                if fixed_count(&collection.constraints)
                    .ok()
                    .flatten()
                    .is_none()
                {
                    continue;
                }
                let mut edges = BTreeSet::new();
                inline_references(&collection.element_type, &header, declarations, &mut edges)?;
                if edges
                    .iter()
                    .any(|target| reaches(target, id, &graph, &mut BTreeSet::new()))
                {
                    result.cyclic.push((id.clone(), collection.clone()));
                }
            }
        }
        Ok(result)
    }
}

fn reaches(
    source: &TypeId,
    target: &TypeId,
    graph: &BTreeMap<TypeId, BTreeSet<TypeId>>,
    visited: &mut BTreeSet<TypeId>,
) -> bool {
    source == target
        || (visited.insert(source.clone())
            && graph.get(source).is_some_and(|edges| {
                edges
                    .iter()
                    .any(|edge| reaches(edge, target, graph, visited))
            }))
}

fn inline_references(
    ty: &ASN1Type,
    header: &ModuleHeader,
    declarations: &TypeDeclarations<'_>,
    output: &mut BTreeSet<TypeId>,
) -> Result<(), GeneratorError> {
    match ty {
        ASN1Type::Jer(_, ty) => inline_references(ty, header, declarations, output)?,
        ASN1Type::ElsewhereDeclaredType(reference) => {
            if let Some(id) = resolve(reference, header, |id| declarations.contains_key(id)) {
                output.insert(id);
            }
        }
        ASN1Type::Sequence(sequence) | ASN1Type::Set(sequence) => {
            for member in &sequence.members {
                if !member.is_recursive {
                    inline_references(&member.ty, header, declarations, output)?;
                }
            }
        }
        ASN1Type::Choice(choice) => {
            for option in &choice.options {
                if !option.is_recursive {
                    inline_references(&option.ty, header, declarations, output)?;
                }
            }
        }
        ASN1Type::SequenceOf(collection)
            if fixed_count(&collection.constraints)
                .ok()
                .flatten()
                .is_some() =>
        {
            if !collection.is_recursive {
                inline_references(&collection.element_type, header, declarations, output)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn collection_candidates<'a>(ty: &'a ASN1Type, output: &mut Vec<&'a SequenceOrSetOf>) {
    match ty {
        ASN1Type::Jer(_, ty) => collection_candidates(ty, output),
        ASN1Type::SequenceOf(collection) => {
            output.push(collection);
            collection_candidates(&collection.element_type, output);
        }
        ASN1Type::SetOf(collection) => collection_candidates(&collection.element_type, output),
        ASN1Type::Sequence(sequence) | ASN1Type::Set(sequence) => {
            for member in &sequence.members {
                collection_candidates(&member.ty, output);
            }
        }
        ASN1Type::Choice(choice) => {
            for option in &choice.options {
                collection_candidates(&option.ty, output);
            }
        }
        _ => {}
    }
}

pub(super) fn fixed_count(constraints: &[Constraint]) -> Result<Option<u32>, GeneratorError> {
    if constraints.is_empty() {
        return Ok(None);
    }
    let mut permitted = vec![(0, i128::MAX)];
    for constraint in constraints {
        let Constraint::Subtype(subtype) = constraint else {
            return Ok(None);
        };
        if subtype.extensible {
            return Ok(None);
        }
        let Some(ranges) = intervals(&subtype.set, false) else {
            return Ok(None);
        };
        if ranges.iter().any(|(min, max)| *min < 0 || max < min) {
            return Err(invalid_size());
        }
        permitted = intersection(&permitted, &ranges);
    }
    if permitted.is_empty() {
        return Err(invalid_size());
    }
    let [(min, max)] = permitted.as_slice() else {
        return Ok(None);
    };
    if min != max {
        return Ok(None);
    }
    let Ok(count) = u32::try_from(*min) else {
        return Ok(None);
    };
    let codec = per_visible_range_constraints(false, constraints)?;
    Ok((!codec.is_extensible()
        && codec.is_size_constraint()
        && codec.min::<i128>() == Some(*min)
        && codec.max::<i128>() == Some(*max))
    .then_some(count))
}

fn invalid_size() -> GeneratorError {
    GeneratorError {
        details: "Invalid or empty collection size constraint".into(),
        kind: GeneratorErrorType::Asn1TypeMismatch,
        ..Default::default()
    }
}

fn integer(value: &ASN1Value) -> Option<i128> {
    match value {
        ASN1Value::Integer(value) | ASN1Value::LinkedIntValue { value, .. } => Some(*value),
        _ => None,
    }
}

fn intervals(set: &ElementOrSetOperation, size: bool) -> Option<Intervals> {
    match set {
        ElementOrSetOperation::Element(element) => element_intervals(element, size),
        ElementOrSetOperation::SetOperation(set) => {
            let left = element_intervals(&set.base, size)?;
            let right = intervals(&set.operant, size)?;
            match set.operator {
                SetOperator::Intersection => Some(intersection(&left, &right)),
                SetOperator::Union => {
                    let mut ranges = left;
                    ranges.extend(right);
                    ranges.sort_unstable();
                    let mut merged: Intervals = Vec::new();
                    for (min, max) in ranges {
                        if let Some(last) = merged.last_mut() {
                            if min <= last.1.saturating_add(1) {
                                last.1 = last.1.max(max);
                                continue;
                            }
                        }
                        merged.push((min, max));
                    }
                    Some(merged)
                }
                SetOperator::Except => None,
            }
        }
    }
}

fn element_intervals(element: &SubtypeElements, size: bool) -> Option<Intervals> {
    match element {
        SubtypeElements::SizeConstraint(set) => intervals(set, true),
        SubtypeElements::SingleValue {
            value,
            extensible: false,
        } if size => {
            let value = integer(value)?;
            Some(vec![(value, value)])
        }
        SubtypeElements::ValueRange {
            min,
            max,
            extensible: false,
        } if size => Some(vec![(
            match min {
                Some(value) => integer(value)?,
                None => 0,
            },
            match max {
                Some(value) => integer(value)?,
                None => i128::MAX,
            },
        )]),
        _ => None,
    }
}

fn intersection(left: &[(i128, i128)], right: &[(i128, i128)]) -> Intervals {
    left.iter()
        .flat_map(|&(a, b)| right.iter().map(move |&(c, d)| (a.max(c), b.min(d))))
        .filter(|(min, max)| min <= max)
        .collect()
}

#[cfg(test)]
mod tests {
    use crate::{CompileResult, Compiler};

    use super::*;

    fn compile(source: &str, enabled: bool) -> Result<CompileResult, crate::error::CompilerError> {
        let mut backend = Rasn::default();
        backend.fixed_size_arrays = enabled;
        Compiler::<Rasn, _>::new()
            .with_backend(backend)
            .add_asn_literal(source)
            .compile_to_string()
    }

    fn output(body: &str, enabled: bool) -> String {
        let result = compile(
            &format!("Test DEFINITIONS AUTOMATIC TAGS ::= BEGIN {body} END"),
            enabled,
        )
        .unwrap();
        assert!(result.warnings.is_empty(), "{:?}", result.warnings);
        result.generated
    }

    #[test]
    fn selects_fixed_sequence_storage() {
        for count in [0, 1, 3, 33, 65_535, 65_536] {
            let body = format!("Flags ::= SEQUENCE SIZE ({count}) OF BOOLEAN");
            let arrays = output(&body, true);
            assert!(
                arrays.contains(&format!("pub struct Flags(pub [AnonymousFlags; {count}]);")),
                "{arrays}"
            );
            assert!(arrays.contains(&format!("size(\"{count}\")")), "{arrays}");
            let vectors = output(&body, false);
            assert!(
                vectors.contains("pub struct Flags(pub SequenceOf<AnonymousFlags>);"),
                "{vectors}"
            );
        }
        let normalized = output(
            "count INTEGER ::= 3 Flags ::= SEQUENCE SIZE (count) OF BOOLEAN",
            true,
        );
        assert!(normalized.contains("[AnonymousFlags; 3]"), "{normalized}");
        let intersection = output(
            "Flags ::= SEQUENCE (SIZE (1..3) INTERSECTION SIZE (3..5)) OF BOOLEAN",
            true,
        );
        assert!(
            intersection.contains("[AnonymousFlags; 3]"),
            "{intersection}"
        );
    }

    #[test]
    fn distinguishes_invalid_and_variable_sizes() {
        for size in ["1..4", "1 | 3", "1..MAX", "3, ...", "4294967296"] {
            let result = output(
                &format!("Flags ::= SEQUENCE SIZE ({size}) OF BOOLEAN"),
                true,
            );
            assert!(
                result.contains("SequenceOf<AnonymousFlags>"),
                "{size}: {result}"
            );
        }
        for size in ["-1", "-1..0", "3..1", "1 INTERSECTION 3"] {
            let result = compile(
                &format!(
                    "Test DEFINITIONS ::= BEGIN Flags ::= SEQUENCE SIZE ({size}) OF BOOLEAN Good ::= BOOLEAN END"
                ),
                true,
            );
            let result = result.unwrap();
            assert_eq!(result.warnings.len(), 1);
            assert!(result.warnings[0].to_string().contains("Flags"), "{:?}", result.warnings);
            assert!(
                result.warnings[0]
                    .to_string()
                    .contains("Invalid or empty collection size"),
                "{:?}",
                result.warnings
            );
            assert!(!result.generated.contains("struct Flags"));
            assert!(result.generated.contains("struct Good"));
        }
    }

    #[test]
    fn keeps_non_sequence_representations() {
        let generated = output(
            "
            Flag ::= BOOLEAN
            Flags ::= SET SIZE (3) OF Flag
            Octets ::= OCTET STRING (SIZE (3))
            Bits ::= BIT STRING (SIZE (3))
            Base ::= SEQUENCE OF Flag
            Three ::= Base (SIZE (3))
        ",
            true,
        );
        for expected in [
            "pub struct Flags(pub SetOf<Flag>);",
            "pub struct Octets(pub FixedOctetString<3usize>);",
            "pub struct Bits(pub FixedBitString<3usize>);",
            "pub struct Base(pub SequenceOf<Flag>);",
            "pub struct Three(pub Base);",
        ] {
            assert!(generated.contains(expected), "{expected}: {generated}");
        }
    }

    #[test]
    fn selects_inline_choice_and_nested_arrays() {
        let generated = output(
            "
            Flag ::= BOOLEAN
            Record ::= SEQUENCE {
                flags [2] EXPLICIT SEQUENCE SIZE (3) OF Flag,
                matrix SEQUENCE SIZE (3) OF SEQUENCE SIZE (2) OF Flag
            }
            Selection ::= CHOICE { flags SEQUENCE SIZE (1) OF Flag, absent NULL }
            Rows ::= SEQUENCE SIZE (2) OF SEQUENCE { flag Flag }
        ",
            true,
        );
        for expected in [
            "[Flag; 3]",
            "[Flag; 1]",
            "[AnonymousRows; 2]",
            "size(\"2\")",
            "size(\"3\")",
        ] {
            assert!(generated.contains(expected), "{expected}: {generated}");
        }
        assert!(
            generated.contains("tag(explicit(context, 2))"),
            "{generated}"
        );
    }

    #[test]
    fn retains_layout_breaks_in_cycles() {
        for count in [0, 1] {
            let generated = output(
                &format!(
                    "
                Node ::= SEQUENCE {{ children SEQUENCE SIZE ({count}) OF Node OPTIONAL }}
                Independent ::= SEQUENCE SIZE ({count}) OF Node
                Flags ::= SEQUENCE SIZE (3) OF BOOLEAN
            "
                ),
                true,
            );
            assert!(generated.contains("SequenceOf<Node>"), "{generated}");
            assert!(generated.contains("[AnonymousFlags; 3]"), "{generated}");
            assert!(
                generated.contains(&format!("pub struct Independent(pub [Node; {count}]);")),
                "{generated}"
            );
        }
        let first = "Left DEFINITIONS ::= BEGIN IMPORTS RightNode FROM Right;
            LeftNode ::= CHOICE { leaf NULL, children SEQUENCE SIZE (1) OF RightNode } END";
        let second = "Right DEFINITIONS ::= BEGIN IMPORTS LeftNode FROM Left;
            RightNode ::= SEQUENCE { children SEQUENCE SIZE (1) OF LeftNode OPTIONAL } END";
        for source in [format!("{first} {second}"), format!("{second} {first}")] {
            let result = compile(&source, true).unwrap();
            assert!(result.warnings.is_empty(), "{:?}", result.warnings);
            assert!(
                !result.generated.contains("[RightNode; 1]"),
                "{}",
                result.generated
            );
            assert!(
                !result.generated.contains("[LeftNode; 1]"),
                "{}",
                result.generated
            );
            assert!(
                result.generated.contains("SequenceOf"),
                "{}",
                result.generated
            );
        }
    }

    #[test]
    fn derives_copy_from_emitted_elements() {
        let generated = output("Flags ::= SEQUENCE SIZE (33) OF BOOLEAN", true);
        let structure = generated.find("pub struct Flags").unwrap();
        let attributes = &generated[..structure];
        assert!(attributes.contains("Copy"), "{generated}");
        let generated = output("Texts ::= SEQUENCE SIZE (3) OF UTF8String", true);
        assert!(!generated.contains("Copy"), "{generated}");
        let mut backend = Rasn::default();
        backend.fixed_size_arrays = true;
        backend.required_derives.push("Default".into());
        let generated = Compiler::<Rasn, _>::new()
            .with_backend(backend)
            .add_asn_literal(
                "Test DEFINITIONS ::= BEGIN Flags ::= SEQUENCE SIZE (33) OF BOOLEAN END",
            )
            .compile_to_string()
            .unwrap()
            .generated;
        assert!(generated.contains("Default"), "{generated}");
        assert!(generated.contains("[AnonymousFlags; 33]"), "{generated}");
    }
}

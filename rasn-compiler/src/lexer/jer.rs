use nom::{
    Parser,
    branch::alt,
    bytes::complete::tag,
    character::complete::char,
    combinator::{cut, map, opt, value},
    multi::{many1, separated_list1},
    sequence::{delimited, preceded},
};

use super::{
    character_string::character_string_value,
    common::{identifier, skip_ws_and_comments, type_reference},
    error::ParserResult,
};
use crate::{
    input::Input,
    intermediate::{ASN1Value, jer::*},
};

pub(super) fn prefix(input: Input<'_>) -> ParserResult<'_, (bool, Instruction)> {
    delimited(
        skip_ws_and_comments(char('[')),
        (
            map(opt((keyword("JER"), skip_ws_and_comments(char(':')))), |prefix| {
                prefix.is_some()
            }),
            instruction,
        ),
        cut(skip_ws_and_comments(char(']'))),
    )
    .parse(input)
}

pub(super) fn control(input: Input<'_>) -> ParserResult<'_, Vec<Assignment>> {
    preceded(
        (keyword("ENCODING-CONTROL"), keyword("JER")),
        cut(many1(map(
            (
                delimited(
                    skip_ws_and_comments(char('[')),
                    instruction,
                    skip_ws_and_comments(char(']')),
                ),
                separated_list1(skip_ws_and_comments(char(',')), target),
            ),
            |(instruction, targets)| Assignment { instruction, targets },
        ))),
    )
    .parse(input)
}

pub(super) fn keyword<'a>(
    name: &'static str,
) -> impl Parser<Input<'a>, Output = Input<'a>, Error = super::error::ErrorTree<'a>> {
    use nom::character::complete::satisfy;
    use nom::combinator::not;
    use nom::sequence::terminated;
    skip_ws_and_comments(terminated(
        tag(name),
        not(satisfy(|character| {
            character.is_ascii_alphanumeric() || character == '-'
        })),
    ))
}

fn instruction(input: Input<'_>) -> ParserResult<'_, Instruction> {
    alt((
        map(preceded(keyword("NOT"), cut(category)), Instruction::Disable),
        map(
            preceded(keyword("NAME"), cut(preceded(keyword("AS"), rename))),
            Instruction::Name,
        ),
        map(
            preceded(
                keyword("TEXT"),
                cut(separated_list1(
                    skip_ws_and_comments(char(',')),
                    (
                        alt((value(None, keyword("ALL")), map(identifier, |id| Some(id.to_owned())))),
                        preceded(keyword("AS"), rename),
                    ),
                )),
            ),
            Instruction::Text,
        ),
        map(
            alt((
                value(Category::Array, keyword("ARRAY")),
                value(Category::Base64, keyword("BASE64")),
                value(Category::Object, keyword("OBJECT")),
                value(Category::Unwrapped, keyword("UNWRAPPED")),
            )),
            Instruction::Enable,
        ),
    ))
    .parse(input)
}

fn category(input: Input<'_>) -> ParserResult<'_, Category> {
    alt((
        value(Category::Array, keyword("ARRAY")),
        value(Category::Base64, keyword("BASE64")),
        value(Category::Name, keyword("NAME")),
        value(Category::Object, keyword("OBJECT")),
        value(Category::Text, keyword("TEXT")),
        value(Category::Unwrapped, keyword("UNWRAPPED")),
    ))
    .parse(input)
}

fn rename(input: Input<'_>) -> ParserResult<'_, Rename> {
    alt((
        map(character_string_value, |value| {
            let ASN1Value::String(text) = value else { unreachable!() };
            Rename::Literal(text)
        }),
        value(Rename::Capitalized, keyword("CAPITALIZED")),
        value(Rename::UpperCamelCased, keyword("UPPERCAMELCASED")),
        value(Rename::UpperCased, keyword("UPPERCASED")),
        value(Rename::LowerCamelCased, keyword("LOWERCAMELCASED")),
        value(Rename::LowerCased, keyword("LOWERCASED")),
        map(super::elsewhere_declared_value, |value| {
            let ASN1Value::ElsewhereDeclaredValue { module, identifier, .. } = value else {
                unreachable!()
            };
            Rename::Reference { module, identifier }
        }),
    ))
    .parse(input)
}

fn target(input: Input<'_>) -> ParserResult<'_, Target> {
    alt((
        map(
            preceded(
                (keyword("ALL"), keyword("IMPORTS"), keyword("FROM")),
                skip_ws_and_comments(type_reference),
            ),
            |module| Target::Imports(module.to_owned()),
        ),
        value(Target::All, keyword("ALL")),
        value(Target::BuiltIn(Category::Array), keyword("SEQUENCE")),
        value(Target::BuiltIn(Category::Base64), (keyword("OCTET"), keyword("STRING"))),
        value(Target::BuiltIn(Category::Object), (keyword("SET"), keyword("OF"))),
        value(Target::BuiltIn(Category::Text), keyword("ENUMERATED")),
        value(Target::BuiltIn(Category::Unwrapped), keyword("CHOICE")),
    ))
    .parse(input)
}

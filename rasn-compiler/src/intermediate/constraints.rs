#[cfg(test)]
use internal_macros::EnumDebug;
use std::error::Error;

use super::{
    error::{GrammarError, GrammarErrorType},
    information_object::{InformationObjectFields, ObjectSet},
    ASN1Type, ASN1Value, IntegerType, RealType,
};

#[derive(Debug, PartialEq)]
pub struct OptionalMarker();

impl From<&str> for OptionalMarker {
    fn from(_: &str) -> Self {
        OptionalMarker()
    }
}

#[derive(Debug)]
pub struct RangeSeperator();

#[derive(Debug, Clone, PartialEq)]
pub struct ExtensionMarker();

/// X.680 49.6 Constraint specification.
///
/// _See X.682 (02/2021) 8_
#[cfg_attr(test, derive(EnumDebug))]
#[cfg_attr(not(test), derive(Debug))]
#[derive(Clone, PartialEq)]
pub enum Constraint {
    Subtype(ElementSetSpecs),
    /// A TableConstraint as specified in X.682 9.
    Table(TableConstraint),
    Parameter(Vec<Parameter>),
    /// A ContentsConstraint as specified in X.682 11.
    Content(ContentConstraint),
}

impl Constraint {
    /// Returns the type of integer that should be used in a representation when applying the
    /// GeneralConstraint.
    pub fn integer_constraints(&self) -> IntegerType {
        let (mut min, mut max, mut is_extensible) = (i128::MAX, i128::MIN, false);
        if let Ok((cmin, cmax, extensible)) = self.unpack_as_value_range() {
            is_extensible = is_extensible || extensible;
            if let Some(ASN1Value::Integer(i)) = cmin {
                min = (*i).min(min);
            };
            if let Some(ASN1Value::Integer(i)) = cmax {
                max = (*i).max(max);
            };
        } else if let Ok((val, extensible)) = self.unpack_as_strict_value() {
            is_extensible = is_extensible || extensible;
            if let ASN1Value::Integer(i) = val {
                min = (*i).min(min);
                max = (*i).max(max);
            };
        };
        if min > max || is_extensible {
            IntegerType::Unbounded
        } else if min >= 0 {
            match max {
                r if r <= u8::MAX.into() => IntegerType::Uint8,
                r if r <= u16::MAX.into() => IntegerType::Uint16,
                r if r <= u32::MAX.into() => IntegerType::Uint32,
                r if r <= u64::MAX.into() => IntegerType::Uint64,
                _ => IntegerType::Unbounded,
            }
        } else {
            match (min, max) {
                (mi, ma) if mi >= i8::MIN.into() && ma <= i8::MAX.into() => IntegerType::Int8,
                (mi, ma) if mi >= i16::MIN.into() && ma <= i16::MAX.into() => IntegerType::Int16,
                (mi, ma) if mi >= i32::MIN.into() && ma <= i32::MAX.into() => IntegerType::Int32,
                (mi, ma) if mi >= i64::MIN.into() && ma <= i64::MAX.into() => IntegerType::Int64,
                _ => IntegerType::Unbounded,
            }
        }
    }

    /// Returns the type of real that should be used in a representation when applying the Constraint
    /// ### Example
    pub fn real_constraints(&self) -> RealType {
        let Ok(RealTypeConstraints {
            mantissa: (mantissa_min, mantissa_max),
            base,
            exponent: (exponent_min, exponent_max),
        }) = self.unpack_as_real_components()
        else {
            return RealType::Unbounded;
        };

        if base == 2
            && exponent_min >= -126
            && exponent_max <= 127
            && mantissa_min >= -9999999
            && mantissa_max <= 9999999
        {
            RealType::F32
        } else if base == 2
            && exponent_min >= -1022
            && exponent_max <= 1023
            && mantissa_min >= -999999999999999
            && mantissa_max <= 999999999999999
        {
            RealType::F64
        } else {
            RealType::Unbounded
        }
    }

    fn unpack_as_real_components(&self) -> Result<RealTypeConstraints, GrammarError> {
        let Constraint::Subtype(ElementSetSpecs {
            set:
                ElementOrSetOperation::Element(SubtypeElements::MultipleTypeConstraints(
                    InnerTypeConstraint {
                        is_partial,
                        constraints,
                    },
                )),
            ..
        }) = self
        else {
            return Err(GrammarError::new(
                &format!(
                    "Failed to unpack constraint as value range. Constraint: {:?}",
                    self
                ),
                GrammarErrorType::UnpackingError,
            ));
        };

        let mut mantissa = None;
        let mut base = None;
        let mut exponent = None;

        for c in constraints {
            match c.identifier.as_str() {
                "mantissa" => {
                    let (from, to) = c.constraints.first().unwrap().unpack_as_value_range()
                        .and_then(|(from, to, _)| {
                            if let (Some(ASN1Value::Integer(from)), Some(ASN1Value::Integer(to))) = (from, to) {
                                Ok((from, to))
                            } else {
                                Err(GrammarError::new(
                                    &format!(
                                        "Failed to unpack constraint as value range. Constraint: {:?}",
                                        self
                                    ),
                                    GrammarErrorType::UnpackingError,
                                ))
                            }
                        })?;

                    mantissa = Some((from.clone(), to.clone()));
                }

                "base" => {
                    let (value, _) = c.constraints.first().unwrap().unpack_as_strict_value()?;
                    base = Some(value.unwrap_as_integer()?);
                }

                "exponent" => {
                    let (from, to) = c.constraints.first().unwrap().unpack_as_value_range()
                        .and_then(|(from, to, _)| {
                            if let (Some(ASN1Value::Integer(from)), Some(ASN1Value::Integer(to))) = (from, to) {
                                Ok((from, to))
                            } else {
                                Err(GrammarError::new(
                                    &format!(
                                        "Failed to unpack constraint as value range. Constraint: {:?}",
                                        self
                                    ),
                                    GrammarErrorType::UnpackingError,
                                ))
                            }
                        })?;

                    exponent = Some((from.clone(), to.clone()));
                }

                _ => (),
            }
        }

        let mantissa = mantissa.ok_or_else(|| {
            GrammarError::new(
                &format!(
                    "Failed to unpack mantissa constraint as value range. Constraint: {:?}",
                    self
                ),
                GrammarErrorType::UnpackingError,
            )
        })?;

        let exponent = exponent.ok_or_else(|| {
            GrammarError::new(
                &format!(
                    "Failed to unpack exponent constraint as value range. Constraint: {:?}",
                    self
                ),
                GrammarErrorType::UnpackingError,
            )
        })?;

        let base = base.ok_or_else(|| {
            GrammarError::new(
                &format!(
                    "Failed to unpack base constraint as value range. Constraint: {:?}",
                    self
                ),
                GrammarErrorType::UnpackingError,
            )
        })?;

        Ok(RealTypeConstraints {
            mantissa,
            base,
            exponent,
        })
    }

    pub fn unpack_as_value_range(
        &self,
    ) -> Result<(&Option<ASN1Value>, &Option<ASN1Value>, bool), GrammarError> {
        if let Constraint::Subtype(set) = self {
            if let ElementOrSetOperation::Element(SubtypeElements::ValueRange {
                min,
                max,
                extensible,
            }) = &set.set
            {
                return Ok((min, max, *extensible));
            }
        }
        Err(GrammarError::new(
            &format!("Failed to unpack constraint as value range. Constraint: {self:?}"),
            GrammarErrorType::UnpackingError,
        ))
    }

    pub fn unpack_as_strict_value(&self) -> Result<(&ASN1Value, bool), GrammarError> {
        if let Constraint::Subtype(set) = self {
            if let ElementOrSetOperation::Element(SubtypeElements::SingleValue {
                value,
                extensible,
            }) = &set.set
            {
                return Ok((value, *extensible));
            }
        }
        Err(GrammarError::new(
            &format!("Failed to unpack constraint as strict value. Constraint: {self:?}"),
            GrammarErrorType::UnpackingError,
        ))
    }
}

struct RealTypeConstraints {
    mantissa: (i128, i128),
    base: i128,
    exponent: (i128, i128),
}

/// A ContentConstraint.
///
/// _See: ITU-T X.682 (02/2021) 11_
#[cfg_attr(test, derive(EnumDebug))]
#[cfg_attr(not(test), derive(Debug))]
#[derive(Clone, PartialEq)]
pub enum ContentConstraint {
    /// **X.682 11.4** _The abstract value of the octet string or bit string is the encoding of an
    /// (any) abstract value of "Type" that is produced by the encoding rules that are applied to
    /// the octet string or bit string._
    Containing(ASN1Type),
    /// **X.682 11.5** _The procedures identified by the object identifier value "Value" shall be
    /// used to produce and to interpret the contents of the bit string or octet string. If the bit
    /// string or octet string is already constrained, it is a specification error if these
    /// procedures do not produce encodings that satisfy the constraint._
    EncodedBy(ASN1Value),
    /// **X.682 11.6** _The abstract value of the octet string or bit string is the encoding of an
    /// (any) abstract value of "Type" that is produced by the encoding rules identified by the
    /// object identifier value "Value"._
    ContainingEncodedBy {
        containing: ASN1Type,
        encoded_by: ASN1Value,
    },
}

#[cfg_attr(test, derive(EnumDebug))]
#[cfg_attr(not(test), derive(Debug))]
#[derive(Clone, PartialEq)]
pub enum Parameter {
    ValueParameter(ASN1Value),
    TypeParameter(ASN1Type),
    InformationObjectParameter(InformationObjectFields),
    ObjectSetParameter(ObjectSet),
}

#[cfg_attr(test, derive(EnumDebug))]
#[cfg_attr(not(test), derive(Debug))]
#[derive(Clone, PartialEq)]
pub enum SetOperator {
    Intersection,
    Union,
    Except,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CompositeConstraint {
    pub base_constraint: Box<Constraint>,
    pub operation: Vec<(SetOperator, Box<Constraint>)>,
    pub extensible: bool,
}

impl
    From<(
        Constraint,
        Vec<(SetOperator, Constraint)>,
        Option<ExtensionMarker>,
    )> for CompositeConstraint
{
    fn from(
        value: (
            Constraint,
            Vec<(SetOperator, Constraint)>,
            Option<ExtensionMarker>,
        ),
    ) -> Self {
        Self {
            base_constraint: Box::new(value.0),
            operation: value
                .1
                .into_iter()
                .map(|(op, c)| (op, Box::new(c)))
                .collect(),
            extensible: value.2.is_some(),
        }
    }
}

#[cfg_attr(test, derive(EnumDebug))]
#[cfg_attr(not(test), derive(Debug))]
#[derive(Clone, PartialEq)]
pub enum ComponentPresence {
    Absent,
    Present,
    Unspecified,
}

/// Representation of a component constraint used for subtyping
/// in ASN1 specifications
#[derive(Debug, Clone, PartialEq)]
pub struct InnerTypeConstraint {
    pub is_partial: bool,
    pub constraints: Vec<NamedConstraint>,
}

/// Representation of a single component within a component constraint
/// in ASN1 specifications
#[derive(Debug, Clone, PartialEq)]
pub struct NamedConstraint {
    pub identifier: String,
    pub constraints: Vec<Constraint>,
    pub presence: ComponentPresence,
}

/// Representation of a range constraint used for subtyping
/// in ASN1 specifications
#[derive(Debug, Clone, PartialEq)]
pub struct ValueConstraint {
    pub min_value: Option<ASN1Value>,
    pub max_value: Option<ASN1Value>,
    pub extensible: bool,
}

impl From<ASN1Value> for ValueConstraint {
    fn from(value: ASN1Value) -> Self {
        Self {
            min_value: Some(value.clone()),
            max_value: Some(value),
            extensible: false,
        }
    }
}

impl From<(ASN1Value, RangeSeperator, ASN1Value)> for ValueConstraint {
    fn from(value: (ASN1Value, RangeSeperator, ASN1Value)) -> Self {
        Self {
            min_value: Some(value.0),
            max_value: Some(value.2),
            extensible: false,
        }
    }
}

impl From<(ASN1Value, ExtensionMarker)> for ValueConstraint {
    fn from(value: (ASN1Value, ExtensionMarker)) -> Self {
        Self {
            min_value: Some(value.0.clone()),
            max_value: Some(value.0),
            extensible: true,
        }
    }
}

impl From<(ASN1Value, RangeSeperator, ASN1Value, ExtensionMarker)> for ValueConstraint {
    fn from(value: (ASN1Value, RangeSeperator, ASN1Value, ExtensionMarker)) -> Self {
        Self {
            min_value: Some(value.0),
            max_value: Some(value.2),
            extensible: true,
        }
    }
}

/// Representation of a table constraint used for subtyping
/// in ASN1 specifications
/// _See: ITU-T X.682 (02/2021) 10_
#[derive(Debug, Clone, PartialEq)]
pub struct TableConstraint {
    pub object_set: ObjectSet,
    pub linked_fields: Vec<RelationalConstraint>,
}

impl From<(ObjectSet, Option<Vec<RelationalConstraint>>)> for TableConstraint {
    fn from(value: (ObjectSet, Option<Vec<RelationalConstraint>>)) -> Self {
        Self {
            object_set: value.0,
            linked_fields: value.1.unwrap_or_default(),
        }
    }
}

/// Representation of a table's relational constraint
/// _See: ITU-T X.682 (02/2021) 10.7_
#[derive(Debug, Clone, PartialEq)]
pub struct RelationalConstraint {
    pub field_name: String,
    /// The level is null if the field is in the outermost object set of the declaration.
    /// The level is 1-n counting from the innermost object set of the declaration
    pub level: usize,
}

impl From<(usize, &str)> for RelationalConstraint {
    fn from(value: (usize, &str)) -> Self {
        Self {
            field_name: value.1.into(),
            level: value.0,
        }
    }
}

/// Representation of a pattern constraint
/// _See: ITU-T X.680 (02/2021) 51.9_
#[derive(Debug, Clone, PartialEq)]
pub struct PatternConstraint {
    pub pattern: String,
}

impl From<&str> for PatternConstraint {
    fn from(value: &str) -> Self {
        Self {
            pattern: value.into(),
        }
    }
}

/// Representation of a user-defined constraint
/// _See: ITU-T X.682 (02/2021) 9_
#[derive(Debug, Clone, PartialEq)]
pub struct UserDefinedConstraint {
    pub definition: String,
}

impl From<&str> for UserDefinedConstraint {
    fn from(value: &str) -> Self {
        Self {
            definition: value.into(),
        }
    }
}

/// Representation of a property settings constraint
/// _See: ITU-T X.680 (02/2021) 51.10_
#[derive(Debug, Clone, PartialEq)]
pub struct PropertySettings {
    pub property_settings_list: Vec<PropertyAndSettingsPair>,
}

impl From<Vec<&str>> for PropertySettings {
    fn from(_value: Vec<&str>) -> Self {
        todo!()
    }
}

#[cfg_attr(test, derive(EnumDebug))]
#[cfg_attr(not(test), derive(Debug))]
#[derive(Clone, PartialEq)]
pub enum PropertyAndSettingsPair {
    Basic(BasicSettings),
    Date(DateSettings),
    Year(YearSettings),
    Time(TimeSettings),
    LocalOrUtc(LocalOrUtcSettings),
    IntervalType(IntervalTypeSettings),
    StartEndPoint(StartEndPointSettings),
    Recurrence(RecurrenceSettings),
    Midnight(MidnightSettings),
}

impl TryFrom<(&str, &str)> for PropertyAndSettingsPair {
    fn try_from(value: (&str, &str)) -> Result<PropertyAndSettingsPair, Box<dyn Error>> {
        match value.0 {
            BasicSettings::NAME => BasicSettings::from_str(value.1).map(Self::Basic),
            DateSettings::NAME => DateSettings::from_str(value.1).map(Self::Date),
            YearSettings::NAME => YearSettings::from_str(value.1).map(Self::Year),
            TimeSettings::NAME => TimeSettings::from_str(value.1).map(Self::Time),
            LocalOrUtcSettings::NAME => LocalOrUtcSettings::from_str(value.1).map(Self::LocalOrUtc),
            IntervalTypeSettings::NAME => {
                IntervalTypeSettings::from_str(value.1).map(Self::IntervalType)
            }
            StartEndPointSettings::NAME => {
                StartEndPointSettings::from_str(value.1).map(Self::StartEndPoint)
            }
            RecurrenceSettings::NAME => RecurrenceSettings::from_str(value.1).map(Self::Recurrence),
            MidnightSettings::NAME => MidnightSettings::from_str(value.1).map(Self::Midnight),
            _ => Err("Unknown Settings value.".into()),
        }
    }

    type Error = Box<dyn Error>;
}

pub trait PropertySetting {
    const NAME: &'static str;

    fn setting_name(&self) -> String;

    fn from_str(value: &str) -> Result<Self, Box<dyn Error>>
    where
        Self: Sized;
}

#[cfg_attr(test, derive(EnumDebug))]
#[cfg_attr(not(test), derive(Debug))]
#[derive(Clone, PartialEq)]
pub enum BasicSettings {
    Date,
    Time,
    DateTime,
    Interval,
    RecInterval,
}

impl PropertySetting for BasicSettings {
    const NAME: &'static str = "Basic";

    fn setting_name(&self) -> String {
        match self {
            BasicSettings::Date => "Date".into(),
            BasicSettings::Time => "Time".into(),
            BasicSettings::DateTime => "Date-Time".into(),
            BasicSettings::Interval => "Interval".into(),
            BasicSettings::RecInterval => "Rec-Interval".into(),
        }
    }

    fn from_str(value: &str) -> Result<Self, Box<dyn Error>> {
        match value {
            "Date" => Ok(BasicSettings::Date),
            "Time" => Ok(BasicSettings::Time),
            "Date-Time" => Ok(BasicSettings::DateTime),
            "Interval" => Ok(BasicSettings::Interval),
            "Rec-Interval" => Ok(BasicSettings::RecInterval),
            _ => Err("Unknown Settings value.".into()),
        }
    }
}

impl PropertySetting for DateSettings {
    const NAME: &'static str = "Date";

    fn setting_name(&self) -> String {
        match self {
            DateSettings::Century => "C".into(),
            DateSettings::Year => "Y".into(),
            DateSettings::YearMonth => "YM".into(),
            DateSettings::YearMonthDay => "YMD".into(),
            DateSettings::YearDay => "YD".into(),
            DateSettings::YearWeek => "YW".into(),
            DateSettings::YearWeekDay => "YWD".into(),
        }
    }

    fn from_str(value: &str) -> Result<Self, Box<dyn Error>> {
        match value {
            "C" => Ok(DateSettings::Century),
            "Y" => Ok(DateSettings::Year),
            "YM" => Ok(DateSettings::YearMonth),
            "YMD" => Ok(DateSettings::YearMonthDay),
            "YD" => Ok(DateSettings::YearDay),
            "YW" => Ok(DateSettings::YearWeek),
            "YWD" => Ok(DateSettings::YearWeekDay),
            _ => Err("Unknown Settings value.".into()),
        }
    }
}

#[cfg_attr(test, derive(EnumDebug))]
#[cfg_attr(not(test), derive(Debug))]
#[derive(Clone, PartialEq)]
pub enum DateSettings {
    Century,
    Year,
    YearMonth,
    YearMonthDay,
    YearDay,
    YearWeek,
    YearWeekDay,
}

impl PropertySetting for YearSettings {
    const NAME: &'static str = "Year";

    fn setting_name(&self) -> String {
        match self {
            YearSettings::Basic => "Basic".into(),
            YearSettings::Proleptic => "Proleptic".into(),
            YearSettings::Negative => "Negative".into(),
            YearSettings::Large(i) => format!("L{i}"),
        }
    }

    fn from_str(value: &str) -> Result<Self, Box<dyn Error>> {
        match value {
            "Basic" => Ok(YearSettings::Basic),
            "Proleptic" => Ok(YearSettings::Proleptic),
            "Negative" => Ok(YearSettings::Negative),
            s if s.starts_with('L') => Ok(s[1..].parse().map(YearSettings::Large)?),
            _ => Err("Unknown Settings value.".into()),
        }
    }
}

#[cfg_attr(test, derive(EnumDebug))]
#[cfg_attr(not(test), derive(Debug))]
#[derive(Clone, PartialEq)]
pub enum YearSettings {
    Basic,
    Proleptic,
    Negative,
    Large(usize),
}

impl PropertySetting for TimeSettings {
    const NAME: &'static str = "Time";

    fn setting_name(&self) -> String {
        match self {
            TimeSettings::Hour => "H".into(),
            TimeSettings::HourMinute => "HM".into(),
            TimeSettings::HourMinuteSecond => "HMS".into(),
            TimeSettings::HourDecimalFraction(i) => format!("HF{i}"),
            TimeSettings::HourMinuteFraction(i) => format!("HMF{i}"),
            TimeSettings::HourMinuteSecondFraction(i) => format!("HMSF{i}"),
        }
    }

    fn from_str(value: &str) -> Result<Self, Box<dyn Error>> {
        match value {
            "H" => Ok(TimeSettings::Hour),
            "HM" => Ok(TimeSettings::HourMinute),
            "HMS" => Ok(TimeSettings::HourMinuteSecond),
            s if s.starts_with("HF") => {
                Ok(s[2..].parse().map(TimeSettings::HourDecimalFraction)?)
            }
            s if s.starts_with("HMF") => {
                Ok(s[3..].parse().map(TimeSettings::HourMinuteFraction)?)
            }
            s if s.starts_with("HMSF") => {
                Ok(s[4..].parse().map(TimeSettings::HourMinuteSecondFraction)?)
            }
            _ => Err("Unknown Settings value.".into()),
        }
    }
}

#[cfg_attr(test, derive(EnumDebug))]
#[cfg_attr(not(test), derive(Debug))]
#[derive(Clone, PartialEq)]
pub enum TimeSettings {
    Hour,
    HourMinute,
    HourMinuteSecond,
    HourDecimalFraction(usize),
    HourMinuteFraction(usize),
    HourMinuteSecondFraction(usize),
}

impl PropertySetting for LocalOrUtcSettings {
    const NAME: &'static str = "Local-or-UTC";

    fn setting_name(&self) -> String {
        match self {
            LocalOrUtcSettings::Local => "L".into(),
            LocalOrUtcSettings::Utc => "Z".into(),
            LocalOrUtcSettings::LocalAndDifference => "LD".into(),
        }
    }

    fn from_str(value: &str) -> Result<Self, Box<dyn Error>> {
        match value {
            "L" => Ok(LocalOrUtcSettings::Local),
            "Z" => Ok(LocalOrUtcSettings::Utc),
            "LD" => Ok(LocalOrUtcSettings::LocalAndDifference),
            _ => Err("Unknown Settings value.".into()),
        }
    }
}

#[cfg_attr(test, derive(EnumDebug))]
#[cfg_attr(not(test), derive(Debug))]
#[derive(Clone, PartialEq)]
pub enum LocalOrUtcSettings {
    Local,
    Utc,
    LocalAndDifference,
}

impl PropertySetting for IntervalTypeSettings {
    const NAME: &'static str = "Interval-type";

    fn setting_name(&self) -> String {
        match self {
            IntervalTypeSettings::StartAndEnd => "SE".into(),
            IntervalTypeSettings::Duration => "D".into(),
            IntervalTypeSettings::StartAndDuration => "SD".into(),
            IntervalTypeSettings::DurationAndEnd => "DE".into(),
        }
    }

    fn from_str(value: &str) -> Result<Self, Box<dyn Error>> {
        match value {
            "SE" => Ok(IntervalTypeSettings::StartAndEnd),
            "D" => Ok(IntervalTypeSettings::Duration),
            "SD" => Ok(IntervalTypeSettings::StartAndDuration),
            "DE" => Ok(IntervalTypeSettings::DurationAndEnd),
            _ => Err("Unknown Settings value.".into()),
        }
    }
}

#[cfg_attr(test, derive(EnumDebug))]
#[cfg_attr(not(test), derive(Debug))]
#[derive(Clone, PartialEq)]
pub enum IntervalTypeSettings {
    StartAndEnd,
    Duration,
    StartAndDuration,
    DurationAndEnd,
}

impl PropertySetting for StartEndPointSettings {
    const NAME: &'static str = "SE-point";

    fn setting_name(&self) -> String {
        match self {
            StartEndPointSettings::Date => "Date".into(),
            StartEndPointSettings::Time => "Time".into(),
            StartEndPointSettings::DateTime => "Date-Time".into(),
        }
    }

    fn from_str(value: &str) -> Result<Self, Box<dyn Error>> {
        match value {
            "Date" => Ok(StartEndPointSettings::Date),
            "Time" => Ok(StartEndPointSettings::Time),
            "Date-Time" => Ok(StartEndPointSettings::DateTime),
            _ => Err("Unknown Settings value.".into()),
        }
    }
}

#[cfg_attr(test, derive(EnumDebug))]
#[cfg_attr(not(test), derive(Debug))]
#[derive(Clone, PartialEq)]
pub enum StartEndPointSettings {
    Date,
    Time,
    DateTime,
}

impl PropertySetting for RecurrenceSettings {
    const NAME: &'static str = "Recurrence";

    fn setting_name(&self) -> String {
        match self {
            RecurrenceSettings::Unlimited => "Unlimited".into(),
            RecurrenceSettings::Recurrences(i) => format!("R{i}"),
        }
    }

    fn from_str(value: &str) -> Result<Self, Box<dyn Error>> {
        match value {
            "Unlimited" => Ok(RecurrenceSettings::Unlimited),
            s if s.starts_with('R') => Ok(s[1..].parse().map(RecurrenceSettings::Recurrences)?),
            _ => Err("Unknown Settings value.".into()),
        }
    }
}

#[cfg_attr(test, derive(EnumDebug))]
#[cfg_attr(not(test), derive(Debug))]
#[derive(Clone, PartialEq)]
pub enum RecurrenceSettings {
    Unlimited,
    Recurrences(usize),
}

impl PropertySetting for MidnightSettings {
    const NAME: &'static str = "Midnight";

    fn setting_name(&self) -> String {
        match self {
            MidnightSettings::StartOfDay => "Start".into(),
            MidnightSettings::EndOfDay => "End".into(),
        }
    }

    fn from_str(value: &str) -> Result<Self, Box<dyn Error>> {
        match value {
            "Start" => Ok(MidnightSettings::StartOfDay),
            "End" => Ok(MidnightSettings::EndOfDay),
            _ => Err("Unknown Settings value.".into()),
        }
    }
}

#[cfg_attr(test, derive(EnumDebug))]
#[cfg_attr(not(test), derive(Debug))]
#[derive(Clone, PartialEq)]
pub enum MidnightSettings {
    StartOfDay,
    EndOfDay,
}

#[cfg_attr(test, derive(EnumDebug))]
#[cfg_attr(not(test), derive(Debug))]
#[derive(Clone, PartialEq)]
pub enum SubtypeElements {
    SingleValue {
        value: ASN1Value,
        extensible: bool,
    },
    ContainedSubtype {
        subtype: ASN1Type,
        extensible: bool,
    },
    ValueRange {
        min: Option<ASN1Value>,
        max: Option<ASN1Value>,
        extensible: bool,
    },
    PermittedAlphabet(Box<ElementOrSetOperation>),
    SizeConstraint(Box<ElementOrSetOperation>),
    TypeConstraint(ASN1Type),
    SingleTypeConstraint(Vec<Constraint>),
    MultipleTypeConstraints(InnerTypeConstraint),
    PatternConstraint(PatternConstraint),
    UserDefinedConstraint(UserDefinedConstraint),
    PropertySettings(PropertySettings), // DurationRange
                                        // TimePointRange
                                        // RecurrenceRange
}

impl From<(ASN1Value, Option<ExtensionMarker>)> for SubtypeElements {
    fn from(value: (ASN1Value, Option<ExtensionMarker>)) -> Self {
        Self::SingleValue {
            value: value.0,
            extensible: value.1.is_some(),
        }
    }
}

impl From<Constraint> for SubtypeElements {
    fn from(value: Constraint) -> Self {
        match value {
            Constraint::Subtype(set) => Self::SizeConstraint(Box::new(set.set)),
            _ => unreachable!(),
        }
    }
}

impl From<(Option<ExtensionMarker>, Vec<NamedConstraint>)> for SubtypeElements {
    fn from(value: (Option<ExtensionMarker>, Vec<NamedConstraint>)) -> Self {
        SubtypeElements::MultipleTypeConstraints(InnerTypeConstraint {
            is_partial: value.0.is_some(),
            constraints: value.1,
        })
    }
}

/// X.680 50. Element set specification
///
/// *50.1* _In some notations a set of elements of some identified type or information object class
/// (the governor) can be specified. In such cases, the notation "ElementSetSpec" is used._
#[derive(Debug, Clone, PartialEq)]
pub struct ElementSetSpecs {
    pub set: ElementOrSetOperation,
    pub extensible: bool,
}

impl From<(ElementOrSetOperation, Option<ExtensionMarker>)> for ElementSetSpecs {
    fn from(value: (ElementOrSetOperation, Option<ExtensionMarker>)) -> Self {
        Self {
            set: value.0,
            extensible: value.1.is_some(),
        }
    }
}

#[cfg_attr(test, derive(EnumDebug))]
#[cfg_attr(not(test), derive(Debug))]
#[derive(Clone, PartialEq)]
pub enum ElementOrSetOperation {
    Element(SubtypeElements),
    SetOperation(SetOperation),
}

#[derive(Debug, Clone, PartialEq)]
pub struct SetOperation {
    pub base: SubtypeElements, //TODO: Handle exclusions
    pub operator: SetOperator,
    pub operant: Box<ElementOrSetOperation>,
}

impl From<(SubtypeElements, SetOperator, ElementOrSetOperation)> for SetOperation {
    fn from(value: (SubtypeElements, SetOperator, ElementOrSetOperation)) -> Self {
        Self {
            base: value.0,
            operator: value.1,
            operant: Box::new(value.2),
        }
    }
}

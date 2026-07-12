//! Implements variables for a shell environment.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fmt::{Display, Write};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::engine::shell::Shell;
use crate::engine::{error, escape};

/// A shell variable.
#[derive(Clone, Debug)]
pub struct ShellVariable {
    /// The value currently associated with the variable.
    value: ShellValue,
    /// Whether or not the variable is marked as exported to child processes.
    exported: bool,
    /// Whether or not the variable is marked as read-only.
    readonly: bool,
    /// Whether or not the variable should be enumerated in the shell's environment.
    enumerable: bool,
    /// The transformation to apply to the variable's value when it is updated.
    transform_on_update: ShellVariableUpdateTransform,
    /// Whether or not the variable is marked as being traced.
    trace: bool,
    /// Whether or not the variable should be treated as an integer.
    treat_as_integer: bool,
    /// Whether or not the variable should be treated as a name reference.
    treat_as_nameref: bool,
    /// 旧式可变引用 API 无法表示的赋值错误.
    assignment_error: Option<&'static str>,
}

/// Kind of transformation to apply to a variable's value when it is updated.
#[derive(Clone, Copy, Debug)]
pub enum ShellVariableUpdateTransform {
    /// No transformation.
    None,
    /// Convert the value to lowercase.
    Lowercase,
    /// Convert the value to uppercase.
    Uppercase,
    /// Convert the value to lowercase, with the first character capitalized.
    Capitalize,
}

impl Default for ShellVariable {
    fn default() -> Self {
        Self {
            value: ShellValue::String(String::new()),
            exported: false,
            readonly: false,
            enumerable: true,
            transform_on_update: ShellVariableUpdateTransform::None,
            trace: false,
            treat_as_integer: false,
            treat_as_nameref: false,
            assignment_error: None,
        }
    }
}

impl ShellVariable {
    /// Returns a new shell variable, initialized with the given value.
    ///
    /// # Arguments
    ///
    /// * `value` - The value to associate with the variable.
    pub fn new<I: Into<ShellValue>>(value: I) -> Self {
        Self {
            value: value.into(),
            ..Self::default()
        }
    }

    /// 创建一个始终拒绝赋值的内部错误变量.
    pub(crate) fn assignment_error(message: &'static str) -> Self {
        Self {
            assignment_error: Some(message),
            ..Self::default()
        }
    }

    /// Returns the value associated with the variable.
    pub const fn value(&self) -> &ShellValue {
        &self.value
    }

    /// Returns whether or not the variable is exported to child processes.
    pub const fn is_exported(&self) -> bool {
        self.exported
    }

    /// Marks the variable as exported to child processes.
    pub const fn export(&mut self) -> &mut Self {
        self.exported = true;
        self
    }

    /// Marks the variable as not exported to child processes.
    pub const fn unexport(&mut self) -> &mut Self {
        self.exported = false;
        self
    }

    /// Returns whether or not the variable is read-only.
    pub const fn is_readonly(&self) -> bool {
        self.readonly
    }

    /// Marks the variable as read-only.
    pub const fn set_readonly(&mut self) -> &mut Self {
        self.readonly = true;
        self
    }

    /// Marks the variable as not read-only.
    pub fn unset_readonly(&mut self) -> Result<&mut Self, error::Error> {
        if self.readonly {
            return Err(error::ErrorKind::ReadonlyVariable.into());
        }

        self.readonly = false;
        Ok(self)
    }

    /// Returns whether or not the variable is traced.
    pub const fn is_trace_enabled(&self) -> bool {
        self.trace
    }

    /// Marks the variable as traced.
    pub const fn enable_trace(&mut self) -> &mut Self {
        self.trace = true;
        self
    }

    /// Marks the variable as not traced.
    pub const fn disable_trace(&mut self) -> &mut Self {
        self.trace = false;
        self
    }

    /// Returns whether or not the variable should be enumerated in the shell's environment.
    pub const fn is_enumerable(&self) -> bool {
        self.enumerable
    }

    /// Marks the variable as not enumerable in the shell's environment.
    pub const fn hide_from_enumeration(&mut self) -> &mut Self {
        self.enumerable = false;
        self
    }

    /// Return the update transform associated with the variable.
    pub const fn get_update_transform(&self) -> ShellVariableUpdateTransform {
        self.transform_on_update
    }

    /// Set the update transform associated with the variable.
    pub const fn set_update_transform(&mut self, transform: ShellVariableUpdateTransform) {
        self.transform_on_update = transform;
    }

    /// Returns whether or not the variable should be treated as an integer.
    pub const fn is_treated_as_integer(&self) -> bool {
        self.treat_as_integer
    }

    /// Marks the variable as being treated as an integer.
    pub const fn treat_as_integer(&mut self) -> &mut Self {
        self.treat_as_integer = true;
        self
    }

    /// Marks the variable as not being treated as an integer.
    pub const fn unset_treat_as_integer(&mut self) -> &mut Self {
        self.treat_as_integer = false;
        self
    }

    /// Returns whether or not the variable should be treated as a name reference.
    pub const fn is_treated_as_nameref(&self) -> bool {
        self.treat_as_nameref
    }

    /// Marks the variable as being treated as a name reference.
    pub const fn treat_as_nameref(&mut self) -> &mut Self {
        self.treat_as_nameref = true;
        self
    }

    /// Marks the variable as not being treated as a name reference.
    pub const fn unset_treat_as_nameref(&mut self) -> &mut Self {
        self.treat_as_nameref = false;
        self
    }

    /// Converts the variable to an indexed array.
    pub fn convert_to_indexed_array(&mut self) -> Result<(), error::Error> {
        match self.value() {
            ShellValue::IndexedArray(_) => Ok(()),
            ShellValue::AssociativeArray(_) => {
                Err(error::ErrorKind::ConvertingAssociativeArrayToIndexedArray.into())
            }
            _ => {
                let mut new_values = BTreeMap::new();
                new_values.insert(
                    0,
                    self.value.to_cow_str_without_dynamic_support().to_string(),
                );
                self.value = ShellValue::IndexedArray(new_values);
                Ok(())
            }
        }
    }

    /// Converts the variable to an associative array.
    pub fn convert_to_associative_array(&mut self) -> Result<(), error::Error> {
        match self.value() {
            ShellValue::AssociativeArray(_) => Ok(()),
            ShellValue::IndexedArray(_) => {
                Err(error::ErrorKind::ConvertingIndexedArrayToAssociativeArray.into())
            }
            _ => {
                let mut new_values: BTreeMap<String, String> = BTreeMap::new();
                new_values.insert(
                    String::from("0"),
                    self.value.to_cow_str_without_dynamic_support().to_string(),
                );
                self.value = ShellValue::AssociativeArray(new_values);
                Ok(())
            }
        }
    }

    /// Assign the given value to the variable, conditionally appending to the preexisting value.
    ///
    /// # Arguments
    ///
    /// * `value` - The value to assign to the variable.
    /// * `append` - Whether or not to append the value to the preexisting value.
    #[expect(clippy::too_many_lines)]
    pub fn assign(&mut self, value: ShellValueLiteral, append: bool) -> Result<(), error::Error> {
        if let Some(message) = self.assignment_error {
            return error::unimp(message);
        }
        if self.is_readonly() {
            return Err(error::ErrorKind::ReadonlyVariable.into());
        }
        let value = self.convert_value_literal_for_assignment(value)?;

        if let ShellValue::Dynamic(dynamic) = &mut self.value {
            return dynamic.assign(value, append);
        }

        if append {
            match (&self.value, &value) {
                // If we're appending an array to a declared-but-unset variable (or appending
                // anything to a declared-but-unset array), then fill it out first.
                (ShellValue::Unset(_), ShellValueLiteral::Array(_))
                | (
                    ShellValue::Unset(
                        ShellValueUnsetType::IndexedArray | ShellValueUnsetType::AssociativeArray,
                    ),
                    _,
                ) => {
                    self.assign(ShellValueLiteral::Array(ArrayLiteral(vec![])), false)?;
                }
                // If we're appending a scalar to a declared-but-unset variable, then
                // start with the empty string. This will result in the right thing happening,
                // even in treat-as-integer cases.
                (ShellValue::Unset(_), ShellValueLiteral::Scalar(_)) => {
                    self.assign(ShellValueLiteral::Scalar(String::new()), false)?;
                }
                // If we're trying to append an array to a string, we first promote the string to be
                // an array with the string being present at index 0.
                (ShellValue::String(_), ShellValueLiteral::Array(_)) => {
                    self.convert_to_indexed_array()?;
                }
                _ => (),
            }

            let treat_as_int = self.is_treated_as_integer();
            let update_transform = self.get_update_transform();

            match &mut self.value {
                ShellValue::String(base) => match value {
                    ShellValueLiteral::Scalar(suffix) => {
                        if treat_as_int {
                            let int_value = parse_assignment_integer(base)?
                                .wrapping_add(parse_assignment_integer(suffix.as_str())?);
                            base.clear();
                            base.push_str(int_value.to_string().as_str());
                        } else {
                            base.push_str(suffix.as_str());
                            Self::apply_value_transforms(base, treat_as_int, update_transform)?;
                        }
                        Ok(())
                    }
                    ShellValueLiteral::Array(_) => {
                        // This case was already handled (see above).
                        Ok(())
                    }
                },
                ShellValue::IndexedArray(existing_values) => match value {
                    ShellValueLiteral::Scalar(new_value) => {
                        self.assign_at_index(String::from("0"), new_value, append)
                    }
                    ShellValueLiteral::Array(new_values) => {
                        ShellValue::update_indexed_array_from_literals(existing_values, new_values);
                        Ok(())
                    }
                },
                ShellValue::AssociativeArray(existing_values) => match value {
                    ShellValueLiteral::Scalar(new_value) => {
                        self.assign_at_index(String::from("0"), new_value, append)
                    }
                    ShellValueLiteral::Array(new_values) => {
                        ShellValue::update_associative_array_from_literals(
                            existing_values,
                            new_values,
                        )
                    }
                },
                ShellValue::Unset(_) => unreachable!("covered in conversion above"),
                ShellValue::Dynamic(_) => unreachable!("dynamic values are handled above"),
            }
        } else {
            match (&self.value, value) {
                // If we're updating an array value with a string, then treat it as an update to
                // just the "0"-indexed element of the array.
                (
                    ShellValue::IndexedArray(_)
                    | ShellValue::AssociativeArray(_)
                    | ShellValue::Unset(
                        ShellValueUnsetType::AssociativeArray | ShellValueUnsetType::IndexedArray,
                    ),
                    ShellValueLiteral::Scalar(s),
                ) => self.assign_at_index(String::from("0"), s, false),

                // If we're updating an indexed array value with an array, then preserve the array
                // type. We also default to using an indexed array if we are
                // assigning an array to a previously string-holding variable.
                (
                    ShellValue::IndexedArray(_)
                    | ShellValue::Unset(
                        ShellValueUnsetType::IndexedArray | ShellValueUnsetType::Untyped,
                    )
                    | ShellValue::String(_),
                    ShellValueLiteral::Array(literal_values),
                ) => {
                    self.value = ShellValue::indexed_array_from_literals(literal_values);
                    Ok(())
                }

                // If we're updating an associative array value with an array, then preserve the
                // array type.
                (
                    ShellValue::AssociativeArray(_)
                    | ShellValue::Unset(ShellValueUnsetType::AssociativeArray),
                    ShellValueLiteral::Array(literal_values),
                ) => {
                    self.value = ShellValue::associative_array_from_literals(literal_values)?;
                    Ok(())
                }

                // Assign a scalar value to a scalar or unset (and untyped) variable.
                (ShellValue::String(_) | ShellValue::Unset(_), ShellValueLiteral::Scalar(s)) => {
                    self.value = ShellValue::String(s);
                    Ok(())
                }
                (ShellValue::Dynamic(_), _) => unreachable!("dynamic values are handled above"),
            }
        }
    }

    /// Assign the given value to the variable at the given index, conditionally appending to the
    /// preexisting value present at that element within the value.
    ///
    /// # Arguments
    ///
    /// * `array_index` - The index at which to assign the value.
    /// * `value` - The value to assign to the variable at the given index.
    /// * `append` - Whether or not to append the value to the preexisting value stored at the given
    ///   index.
    pub fn assign_at_index(
        &mut self,
        array_index: String,
        value: String,
        append: bool,
    ) -> Result<(), error::Error> {
        if let Some(message) = self.assignment_error {
            return error::unimp(message);
        }
        if self.is_readonly() {
            return Err(error::ErrorKind::ReadonlyVariable.into());
        }

        let value = self.convert_value_str_for_assignment(value)?;
        if let ShellValue::Dynamic(dynamic) = &mut self.value {
            if array_index.parse::<i64>().unwrap_or(0) != 0 {
                return error::unimp("assigning a nonzero index of a dynamic scalar variable");
            }
            return dynamic.assign(ShellValueLiteral::Scalar(value), append);
        }

        match &self.value {
            ShellValue::Unset(_) => {
                self.assign(ShellValueLiteral::Array(ArrayLiteral(vec![])), false)?;
            }
            ShellValue::String(_) => {
                self.convert_to_indexed_array()?;
            }
            _ => (),
        }

        let treat_as_int = self.is_treated_as_integer();

        match &mut self.value {
            ShellValue::IndexedArray(arr) => {
                let key = get_key_for_indexed_array(arr, array_index.as_str())?;

                if append {
                    let existing_value = arr.get(&key).map_or_else(|| "", |v| v.as_str());

                    let mut new_value;
                    if treat_as_int {
                        new_value = parse_assignment_integer(existing_value)?
                            .wrapping_add(parse_assignment_integer(value.as_str())?)
                            .to_string();
                    } else {
                        new_value = existing_value.to_owned();
                        new_value.push_str(value.as_str());
                    }

                    arr.insert(key, new_value);
                } else {
                    arr.insert(key, value);
                }

                Ok(())
            }
            ShellValue::AssociativeArray(arr) => {
                if append {
                    let existing_value = arr
                        .get(array_index.as_str())
                        .map_or_else(|| "", |v| v.as_str());

                    let mut new_value;
                    if treat_as_int {
                        new_value = parse_assignment_integer(existing_value)?
                            .wrapping_add(parse_assignment_integer(value.as_str())?)
                            .to_string();
                    } else {
                        new_value = existing_value.to_owned();
                        new_value.push_str(value.as_str());
                    }

                    arr.insert(array_index, new_value);
                } else {
                    arr.insert(array_index, value);
                }
                Ok(())
            }
            _ => {
                log::error!("assigning to index {array_index} of {:?}", self.value);
                error::unimp("assigning to index of non-array variable")
            }
        }
    }

    fn convert_value_literal_for_assignment(
        &self,
        value: ShellValueLiteral,
    ) -> Result<ShellValueLiteral, error::Error> {
        match value {
            ShellValueLiteral::Scalar(s) => Ok(ShellValueLiteral::Scalar(
                self.convert_value_str_for_assignment(s)?,
            )),
            ShellValueLiteral::Array(literals) => Ok(ShellValueLiteral::Array(ArrayLiteral(
                literals
                    .0
                    .into_iter()
                    .map(|(k, v)| Ok((k, self.convert_value_str_for_assignment(v)?)))
                    .collect::<Result<Vec<_>, error::Error>>()?,
            ))),
        }
    }

    fn convert_value_str_for_assignment(&self, mut s: String) -> Result<String, error::Error> {
        Self::apply_value_transforms(
            &mut s,
            self.is_treated_as_integer(),
            self.get_update_transform(),
        )?;

        Ok(s)
    }

    fn apply_value_transforms(
        s: &mut String,
        treat_as_int: bool,
        update_transform: ShellVariableUpdateTransform,
    ) -> Result<(), error::Error> {
        if treat_as_int {
            *s = parse_assignment_integer(s)?.to_string();
        } else {
            match update_transform {
                ShellVariableUpdateTransform::None => (),
                ShellVariableUpdateTransform::Lowercase => *s = (*s).to_lowercase(),
                ShellVariableUpdateTransform::Uppercase => *s = (*s).to_uppercase(),
                ShellVariableUpdateTransform::Capitalize => {
                    // This isn't really title-case; only the first character is capitalized.
                    *s = s.to_lowercase();
                    if let Some(c) = s.chars().next() {
                        let first_char_len = c.len_utf8();
                        s.replace_range(0..first_char_len, &c.to_uppercase().to_string());
                    }
                }
            }
        }
        Ok(())
    }

    /// Tries to unset the value stored at the given index in the variable. Returns
    /// whether or not a value was unset.
    ///
    /// # Arguments
    ///
    /// * `index` - The index at which to unset the value.
    pub fn unset_index(&mut self, index: &str) -> Result<bool, error::Error> {
        match &mut self.value {
            ShellValue::Unset(ty) => match ty {
                ShellValueUnsetType::Untyped => Err(error::ErrorKind::NotArray.into()),
                ShellValueUnsetType::AssociativeArray | ShellValueUnsetType::IndexedArray => {
                    Ok(false)
                }
            },
            ShellValue::String(_) => Err(error::ErrorKind::NotArray.into()),
            ShellValue::AssociativeArray(values) => Ok(values.remove(index).is_some()),
            ShellValue::IndexedArray(values) => {
                let key = get_key_for_indexed_array(values, index)?;
                Ok(values.remove(&key).is_some())
            }
            ShellValue::Dynamic(_) => Ok(false),
        }
    }

    /// Returns the variable's value; for dynamic values, this will resolve the value.
    ///
    /// # Arguments
    ///
    /// * `shell` - The shell in which the variable is being resolved.
    pub fn resolve_value(&self, shell: &Shell) -> ShellValue {
        // N.B. We do *not* specially handle a dynamic value that resolves to a dynamic value.
        match &self.value {
            ShellValue::Dynamic(dynamic) => dynamic.resolve(shell),
            _ => self.value.clone(),
        }
    }

    /// Returns the canonical attribute flag string for this variable.
    pub fn attribute_flags(&self, shell: &Shell) -> String {
        let value = self.resolve_value(shell);

        let mut result = String::new();

        if matches!(
            value,
            ShellValue::IndexedArray(_) | ShellValue::Unset(ShellValueUnsetType::IndexedArray)
        ) {
            result.push('a');
        }
        if matches!(
            value,
            ShellValue::AssociativeArray(_)
                | ShellValue::Unset(ShellValueUnsetType::AssociativeArray)
        ) {
            result.push('A');
        }
        if matches!(
            self.get_update_transform(),
            ShellVariableUpdateTransform::Capitalize
        ) {
            result.push('c');
        }
        if self.is_treated_as_integer() {
            result.push('i');
        }
        if self.is_treated_as_nameref() {
            result.push('n');
        }
        if self.is_readonly() {
            result.push('r');
        }
        if matches!(
            self.get_update_transform(),
            ShellVariableUpdateTransform::Lowercase
        ) {
            result.push('l');
        }
        if self.is_trace_enabled() {
            result.push('t');
        }
        if matches!(
            self.get_update_transform(),
            ShellVariableUpdateTransform::Uppercase
        ) {
            result.push('u');
        }
        if self.is_exported() {
            result.push('x');
        }

        result
    }
}

/// 可动态解析的 shell 变量种类.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DynamicVariable {
    BashOpts,
    BashAliases,
    BashArgc,
    BashArgv,
    BashArgv0,
    BashCmds,
    BashLineno,
    BashSource,
    BashSubshell,
    DirStack,
    EpochRealtime,
    EpochSeconds,
    FuncName,
    Groups,
    HistCmd,
    LineNo,
    PipeStatus,
    Random,
    Seconds,
    ShellOpts,
    SRandom,
}

const RANDOM_MULTIPLIER: u64 = 6_364_136_223_846_793_005;
const RANDOM_INCREMENT: u64 = 1_442_695_040_888_963_407;

#[derive(Debug)]
enum DynamicValueState {
    Stateless,
    Random(Mutex<Option<u64>>),
    SRandom(Mutex<Option<u64>>),
    Seconds {
        baseline: SystemTime,
        offset: i64,
    },
    BashArgv0 {
        value: Option<String>,
        suffix: String,
    },
}

impl Clone for DynamicValueState {
    fn clone(&self) -> Self {
        match self {
            Self::Stateless => Self::Stateless,
            Self::Random(state) | Self::SRandom(state) => {
                let state = state
                    .lock()
                    .map_or_else(|poisoned| *poisoned.into_inner(), |state| *state);
                match self {
                    Self::Random(_) => Self::Random(Mutex::new(state)),
                    Self::SRandom(_) => Self::SRandom(Mutex::new(state)),
                    _ => unreachable!(),
                }
            }
            Self::Seconds { baseline, offset } => Self::Seconds {
                baseline: *baseline,
                offset: *offset,
            },
            Self::BashArgv0 { value, suffix } => Self::BashArgv0 {
                value: value.clone(),
                suffix: suffix.clone(),
            },
        }
    }
}

/// 动态解析且可携带独立运行状态的 shell 值.
#[derive(Clone, Debug)]
pub struct DynamicShellValue {
    variable: DynamicVariable,
    state: DynamicValueState,
}

impl DynamicShellValue {
    /// 创建指定种类的动态 shell 值.
    pub fn new(variable: DynamicVariable) -> Self {
        let state = match variable {
            DynamicVariable::Random => DynamicValueState::Random(Mutex::new(None)),
            DynamicVariable::SRandom => DynamicValueState::SRandom(Mutex::new(None)),
            DynamicVariable::Seconds => DynamicValueState::Seconds {
                baseline: SystemTime::now(),
                offset: 0,
            },
            DynamicVariable::BashArgv0 => DynamicValueState::BashArgv0 {
                value: None,
                suffix: String::new(),
            },
            _ => DynamicValueState::Stateless,
        };
        Self { variable, state }
    }

    /// 返回动态值的种类.
    pub const fn variable(&self) -> DynamicVariable {
        self.variable
    }

    pub(crate) fn assign(
        &mut self,
        value: ShellValueLiteral,
        append: bool,
    ) -> Result<(), error::Error> {
        let ShellValueLiteral::Scalar(value) = value else {
            return error::unimp("assigning an array to a dynamic variable");
        };

        match &mut self.state {
            DynamicValueState::Random(state) => {
                let assigned = value.parse::<i64>().unwrap_or(0);
                let seed = if append {
                    let current = i64::try_from(Self::next_random(state) % 32_768).unwrap_or(0);
                    current.wrapping_add(assigned)
                } else {
                    assigned
                };
                let seed = u64::from_ne_bytes(seed.to_ne_bytes());
                *state
                    .get_mut()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(seed);
                Ok(())
            }
            DynamicValueState::Seconds { baseline, offset } => {
                let assigned = value.parse::<i64>().unwrap_or(0);
                if append {
                    *offset = Self::elapsed_seconds(*baseline).saturating_add(*offset);
                    *offset = offset.wrapping_add(assigned);
                } else {
                    *offset = assigned;
                }
                *baseline = SystemTime::now();
                Ok(())
            }
            DynamicValueState::BashArgv0 {
                value: current,
                suffix,
            } => {
                if append {
                    if let Some(current) = current {
                        current.push_str(&value);
                    } else {
                        suffix.push_str(&value);
                    }
                } else {
                    *current = Some(value);
                    suffix.clear();
                }
                Ok(())
            }
            DynamicValueState::Stateless | DynamicValueState::SRandom(_) => {
                error::unimp("assignment is unsupported for this dynamic variable")
            }
        }
    }

    pub(crate) fn next_random_value(&self) -> u64 {
        let DynamicValueState::Random(state) = &self.state else {
            return 0;
        };
        Self::next_random(state) % 32_768
    }

    pub(crate) fn next_srandom_value(&self) -> u32 {
        let DynamicValueState::SRandom(state) = &self.state else {
            return 0;
        };
        let bytes = Self::next_random(state).to_ne_bytes();
        u32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
    }

    pub(crate) fn seconds_value(&self) -> i64 {
        let DynamicValueState::Seconds { baseline, offset } = &self.state else {
            return 0;
        };
        Self::elapsed_seconds(*baseline).saturating_add(*offset)
    }

    pub(crate) fn bash_argv0_override(&self) -> Option<&str> {
        let DynamicValueState::BashArgv0 { value, .. } = &self.state else {
            return None;
        };
        value.as_deref()
    }

    pub(crate) fn bash_argv0_suffix(&self) -> Option<&str> {
        let DynamicValueState::BashArgv0 { suffix, .. } = &self.state else {
            return None;
        };
        Some(suffix)
    }

    fn next_random(state: &Mutex<Option<u64>>) -> u64 {
        let mut state = state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let current = state.unwrap_or_else(initial_random_seed);
        let next = current
            .wrapping_mul(RANDOM_MULTIPLIER)
            .wrapping_add(RANDOM_INCREMENT);
        *state = Some(next);
        next
    }

    fn elapsed_seconds(baseline: SystemTime) -> i64 {
        let seconds = SystemTime::now()
            .duration_since(baseline)
            .unwrap_or_default()
            .as_secs();
        i64::try_from(seconds).unwrap_or(i64::MAX)
    }
}

fn initial_random_seed() -> u64 {
    let time_seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            duration.as_secs() ^ u64::from(duration.subsec_nanos()).rotate_left(32)
        });

    (time_seed ^ u64::from(std::process::id())).max(1)
}

/// A shell value.
#[derive(Clone, Debug)]
pub enum ShellValue {
    /// A value that has been typed but not yet set.
    Unset(ShellValueUnsetType),
    /// A string.
    String(String),
    /// An associative array.
    AssociativeArray(BTreeMap<String, String>),
    /// An indexed array.
    IndexedArray(BTreeMap<u64, String>),
    /// A value that is dynamically computed.
    Dynamic(DynamicShellValue),
}

/// The type of an unset shell value.
#[derive(Clone, Debug)]
pub enum ShellValueUnsetType {
    /// The value is untyped.
    Untyped,
    /// The value is an associative array.
    AssociativeArray,
    /// The value is an indexed array.
    IndexedArray,
}

/// A shell value literal; used for assignment.
#[derive(Clone, Debug)]
pub enum ShellValueLiteral {
    /// A scalar value.
    Scalar(String),
    /// An array value.
    Array(ArrayLiteral),
}

impl ShellValueLiteral {
    pub(crate) fn fmt_for_tracing(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Scalar(s) => Self::fmt_scalar_for_tracing(s.as_str(), f),
            Self::Array(elements) => {
                write!(f, "(")?;
                for (i, (key, value)) in elements.0.iter().enumerate() {
                    if i > 0 {
                        write!(f, " ")?;
                    }
                    if let Some(key) = key {
                        write!(f, "[")?;
                        Self::fmt_scalar_for_tracing(key.as_str(), f)?;
                        write!(f, "]=")?;
                    }
                    Self::fmt_scalar_for_tracing(value.as_str(), f)?;
                }
                write!(f, ")")
            }
        }
    }

    fn fmt_scalar_for_tracing(s: &str, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let processed = escape::quote_if_needed(s, escape::QuoteMode::SingleQuote);
        write!(f, "{processed}")
    }
}

impl Display for ShellValueLiteral {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.fmt_for_tracing(f)
    }
}

impl From<&str> for ShellValueLiteral {
    fn from(value: &str) -> Self {
        Self::Scalar(value.to_owned())
    }
}

impl From<String> for ShellValueLiteral {
    fn from(value: String) -> Self {
        Self::Scalar(value)
    }
}

impl From<Vec<&str>> for ShellValueLiteral {
    fn from(value: Vec<&str>) -> Self {
        Self::Array(ArrayLiteral(
            value.into_iter().map(|s| (None, s.to_owned())).collect(),
        ))
    }
}

/// An array literal.
#[derive(Clone, Debug)]
pub struct ArrayLiteral(pub Vec<(Option<String>, String)>);

/// Style for formatting a shell variable's value.
#[derive(Copy, Clone, Debug)]
pub enum FormatStyle {
    /// Basic formatting.
    Basic,
    /// Formatting as appropriate in the `declare` built-in command.
    DeclarePrint,
}

impl ShellValue {
    /// Returns whether or not the value is an array.
    pub const fn is_array(&self) -> bool {
        matches!(
            self,
            Self::IndexedArray(_)
                | Self::AssociativeArray(_)
                | Self::Unset(
                    ShellValueUnsetType::IndexedArray | ShellValueUnsetType::AssociativeArray
                )
        )
    }

    /// Returns whether or not the value is set.
    pub const fn is_set(&self) -> bool {
        !matches!(self, Self::Unset(_))
    }

    /// Returns a new indexed array value constructed from the given slice of owned strings.
    ///
    /// # Arguments
    ///
    /// * `values` - The slice of strings to construct the indexed array from.
    pub fn indexed_array_from_strings<S>(values: S) -> Self
    where
        S: IntoIterator<Item = String>,
    {
        let mut owned_values = BTreeMap::new();
        for (i, value) in values.into_iter().enumerate() {
            owned_values.insert(i as u64, value);
        }

        Self::IndexedArray(owned_values)
    }

    /// Returns a new indexed array value constructed from the given slice of unowned strings.
    ///
    /// # Arguments
    ///
    /// * `values` - The slice of strings to construct the indexed array from.
    pub fn indexed_array_from_strs(values: &[&str]) -> Self {
        let mut owned_values = BTreeMap::new();
        for (i, value) in values.iter().enumerate() {
            owned_values.insert(i as u64, (*value).to_string());
        }

        Self::IndexedArray(owned_values)
    }

    /// Returns a new indexed array value constructed from the given literals.
    ///
    /// # Arguments
    ///
    /// * `literals` - The literals to construct the indexed array from.
    pub fn indexed_array_from_literals(literals: ArrayLiteral) -> Self {
        let mut values = BTreeMap::new();
        Self::update_indexed_array_from_literals(&mut values, literals);

        Self::IndexedArray(values)
    }

    fn update_indexed_array_from_literals(
        existing_values: &mut BTreeMap<u64, String>,
        literal_values: ArrayLiteral,
    ) {
        let mut new_key = if let Some((largest_index, _)) = existing_values.last_key_value() {
            largest_index.saturating_add(1)
        } else {
            0
        };

        for (key, value) in literal_values.0 {
            if let Some(key) = key {
                new_key = key.parse().unwrap_or(0);
            }

            existing_values.insert(new_key, value);
            new_key = new_key.saturating_add(1);
        }
    }

    /// Returns a new associative array value constructed from the given literals.
    ///
    /// # Arguments
    ///
    /// * `literals` - The literals to construct the associative array from.
    pub fn associative_array_from_literals(literals: ArrayLiteral) -> Result<Self, error::Error> {
        let mut values = BTreeMap::new();
        Self::update_associative_array_from_literals(&mut values, literals)?;

        Ok(Self::AssociativeArray(values))
    }

    fn update_associative_array_from_literals(
        existing_values: &mut BTreeMap<String, String>,
        literal_values: ArrayLiteral,
    ) -> Result<(), error::Error> {
        let mut current_key = None;
        for (key, value) in literal_values.0 {
            if let Some(current_key) = current_key.take() {
                if key.is_some() {
                    return error::unimp("misaligned keys/values in associative array literal");
                } else {
                    existing_values.insert(current_key, value);
                }
            } else if let Some(key) = key {
                existing_values.insert(key, value);
            } else {
                current_key = Some(value);
            }
        }

        if let Some(current_key) = current_key {
            existing_values.insert(current_key, String::new());
        }

        Ok(())
    }

    /// Formats the value using the given style.
    ///
    /// # Arguments
    ///
    /// * `style` - The style to use for formatting the value.
    pub fn format(&self, style: FormatStyle, shell: &Shell) -> Result<Cow<'_, str>, error::Error> {
        match self {
            Self::Unset(_) => Ok("".into()),
            Self::String(s) => match style {
                FormatStyle::Basic => Ok(escape::quote_if_needed(
                    s.as_str(),
                    escape::QuoteMode::SingleQuote,
                )),
                FormatStyle::DeclarePrint => {
                    Ok(escape::force_quote(s.as_str(), escape::QuoteMode::DoubleQuote).into())
                }
            },
            Self::AssociativeArray(values) => {
                let mut result = String::new();
                result.push('(');

                for (key, value) in values {
                    let formatted_key =
                        escape::quote_if_needed(key.as_str(), escape::QuoteMode::DoubleQuote);
                    let formatted_value =
                        escape::force_quote(value.as_str(), escape::QuoteMode::DoubleQuote);

                    // N.B. We include an unconditional trailing space character (even after the
                    // last entry in the associative array) to match standard
                    // output behavior.
                    write!(result, "[{formatted_key}]={formatted_value} ")?;
                }

                result.push(')');
                Ok(result.into())
            }
            Self::IndexedArray(values) => {
                let mut result = String::new();
                result.push('(');

                for (i, (key, value)) in values.iter().enumerate() {
                    if i > 0 {
                        result.push(' ');
                    }

                    let formatted_value =
                        escape::force_quote(value.as_str(), escape::QuoteMode::DoubleQuote);
                    write!(result, "[{key}]={formatted_value}")?;
                }

                result.push(')');
                Ok(result.into())
            }
            Self::Dynamic(dynamic) => {
                let dynamic_value = dynamic.resolve(shell);
                let result = dynamic_value.format(style, shell)?.to_string();
                Ok(result.into())
            }
        }
    }

    /// Tries to retrieve the value stored at the given index in this variable.
    ///
    /// # Arguments
    ///
    /// * `index` - The index at which to retrieve the value.
    pub fn get_at(&self, index: &str, shell: &Shell) -> Result<Option<Cow<'_, str>>, error::Error> {
        match self {
            Self::Unset(_) => Ok(None),
            Self::String(s) => {
                if index.parse::<u64>().unwrap_or(0) == 0 {
                    Ok(Some(Cow::Borrowed(s)))
                } else {
                    Ok(None)
                }
            }
            Self::AssociativeArray(values) => {
                Ok(values.get(index).map(|s| Cow::Borrowed(s.as_str())))
            }
            Self::IndexedArray(values) => {
                let key = get_key_for_indexed_array(values, index)?;
                Ok(values.get(&key).map(|s| Cow::Borrowed(s.as_str())))
            }
            Self::Dynamic(dynamic) => {
                let dynamic_value = dynamic.resolve(shell);
                let result = dynamic_value.get_at(index, shell)?;
                Ok(result.map(|s| s.to_string().into()))
            }
        }
    }

    /// Returns the keys of the elements in this variable.
    pub fn element_keys(&self, shell: &Shell) -> Vec<String> {
        match self {
            Self::Unset(_) => vec![],
            Self::String(_) => vec!["0".to_owned()],
            Self::AssociativeArray(array) => array.keys().map(|k| k.to_owned()).collect(),
            Self::IndexedArray(array) => array.keys().map(|k| k.to_string()).collect(),
            Self::Dynamic(dynamic) => dynamic.resolve(shell).element_keys(shell),
        }
    }

    /// Returns the values of the elements in this variable.
    pub fn element_values(&self, shell: &Shell) -> Vec<String> {
        match self {
            Self::Unset(_) => vec![],
            Self::String(s) => vec![s.to_owned()],
            Self::AssociativeArray(array) => array.values().map(|v| v.to_owned()).collect(),
            Self::IndexedArray(array) => array.values().map(|v| v.to_owned()).collect(),
            Self::Dynamic(dynamic) => dynamic.resolve(shell).element_values(shell),
        }
    }

    /// Converts this value to a string.
    pub fn to_cow_str(&self, shell: &Shell) -> Cow<'_, str> {
        self.try_get_cow_str(shell).unwrap_or(Cow::Borrowed(""))
    }

    fn to_cow_str_without_dynamic_support(&self) -> Cow<'_, str> {
        self.try_get_cow_str_without_dynamic_support()
            .unwrap_or(Cow::Borrowed(""))
    }

    /// Tries to convert this value to a string; returns `None` if the value is unset
    /// or otherwise doesn't exist.
    pub fn try_get_cow_str(&self, shell: &Shell) -> Option<Cow<'_, str>> {
        match self {
            Self::Dynamic(dynamic) => {
                let dynamic_value = dynamic.resolve(shell);
                dynamic_value
                    .try_get_cow_str(shell)
                    .map(|s| s.to_string().into())
            }
            _ => self.try_get_cow_str_without_dynamic_support(),
        }
    }

    fn try_get_cow_str_without_dynamic_support(&self) -> Option<Cow<'_, str>> {
        match self {
            Self::Unset(_) => None,
            Self::String(s) => Some(Cow::Borrowed(s.as_str())),
            Self::AssociativeArray(values) => values.get("0").map(|s| Cow::Borrowed(s.as_str())),
            Self::IndexedArray(values) => values.get(&0).map(|s| Cow::Borrowed(s.as_str())),
            Self::Dynamic(_) => None,
        }
    }

    /// Formats this value as a program string usable in an assignment.
    ///
    /// # Arguments
    ///
    /// * `index` - The index at which to retrieve the value, if indexing is to be performed.
    pub fn to_assignable_str(
        &self,
        index: Option<&str>,
        shell: &Shell,
    ) -> Result<String, error::Error> {
        match self {
            Self::Unset(_) => Ok(String::new()),
            Self::String(s) => Ok(escape::force_quote(
                s.as_str(),
                escape::QuoteMode::SingleQuote,
            )),
            Self::AssociativeArray(_) | Self::IndexedArray(_) => {
                if let Some(index) = index {
                    if let Ok(Some(value)) = self.get_at(index, shell) {
                        Ok(escape::force_quote(
                            value.as_ref(),
                            escape::QuoteMode::SingleQuote,
                        ))
                    } else {
                        Ok(String::new())
                    }
                } else {
                    Ok(self.format(FormatStyle::DeclarePrint, shell)?.into_owned())
                }
            }
            Self::Dynamic(dynamic) => dynamic.resolve(shell).to_assignable_str(index, shell),
        }
    }
}

fn parse_assignment_integer(value: &str) -> Result<i64, error::Error> {
    if value.is_empty() {
        return Ok(0);
    }
    value.parse::<i64>().map_err(|inner| {
        error::ErrorKind::IntParseError {
            s: value.to_owned(),
            int_type_name: "i64",
            radix: 10,
            inner,
        }
        .into()
    })
}

fn get_key_for_indexed_array(
    values: &BTreeMap<u64, String>,
    index_str: &str,
) -> Result<u64, error::Error> {
    let index_value = index_str.parse::<i64>().unwrap_or(0);

    if index_value < 0 {
        let Some((&largest_index, _)) = values.last_key_value() else {
            return Err(error::ErrorKind::ArrayIndexOutOfRange(index_str.to_owned()).into());
        };
        let offset_from_largest = index_value.unsigned_abs() - 1;
        return largest_index
            .checked_sub(offset_from_largest)
            .ok_or_else(|| error::ErrorKind::ArrayIndexOutOfRange(index_str.to_owned()).into());
    }

    #[expect(clippy::cast_sign_loss)]
    Ok(index_value as u64)
}

impl From<DynamicVariable> for ShellValue {
    fn from(value: DynamicVariable) -> Self {
        Self::Dynamic(DynamicShellValue::new(value))
    }
}

impl From<&str> for ShellValue {
    fn from(value: &str) -> Self {
        Self::String(value.to_owned())
    }
}

impl From<&String> for ShellValue {
    fn from(value: &String) -> Self {
        Self::String(value.clone())
    }
}

impl From<String> for ShellValue {
    fn from(value: String) -> Self {
        Self::String(value)
    }
}

impl From<OsString> for ShellValue {
    fn from(value: OsString) -> Self {
        Self::String(value.to_string_lossy().into_owned())
    }
}

impl From<Vec<String>> for ShellValue {
    fn from(values: Vec<String>) -> Self {
        Self::indexed_array_from_strings(values)
    }
}

impl From<Vec<&str>> for ShellValue {
    fn from(values: Vec<&str>) -> Self {
        Self::indexed_array_from_strs(values.as_slice())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn capitalize_handles_multibyte_first_characters() {
        let mut variable = ShellVariable::new("");
        variable.set_update_transform(ShellVariableUpdateTransform::Capitalize);

        variable
            .assign(ShellValueLiteral::Scalar("éCLAIR".to_owned()), false)
            .unwrap();
        assert!(matches!(variable.value(), ShellValue::String(value) if value == "Éclair"));

        variable
            .assign(ShellValueLiteral::Scalar("ßTRAẞE".to_owned()), false)
            .unwrap();
        assert!(matches!(variable.value(), ShellValue::String(value) if value == "SStraße"));
    }

    #[test]
    fn indexed_array_negative_indices_are_relative_to_largest_index() {
        let values = BTreeMap::from([(2, "two".to_owned()), (7, "seven".to_owned())]);

        assert_eq!(get_key_for_indexed_array(&values, "-1").unwrap(), 7);
        assert_eq!(get_key_for_indexed_array(&values, "-2").unwrap(), 6);
        assert_eq!(get_key_for_indexed_array(&values, "-8").unwrap(), 0);
        assert!(get_key_for_indexed_array(&values, "-9").is_err());
        assert!(get_key_for_indexed_array(&BTreeMap::new(), "-1").is_err());
    }

    #[test]
    fn readonly_dynamic_variables_return_readonly_error() {
        let mut variable = ShellVariable::new(DynamicVariable::BashOpts);
        variable.set_readonly();

        let error = variable
            .assign(ShellValueLiteral::Scalar("1".to_owned()), false)
            .unwrap_err();
        assert!(matches!(error.kind(), error::ErrorKind::ReadonlyVariable));

        let error = variable
            .assign_at_index("0".to_owned(), "1".to_owned(), false)
            .unwrap_err();
        assert!(matches!(error.kind(), error::ErrorKind::ReadonlyVariable));
    }

    #[test]
    fn integer_append_uses_wrapping_addition() {
        let mut scalar = ShellVariable::new(i64::MAX.to_string());
        scalar.treat_as_integer();
        scalar
            .assign(ShellValueLiteral::Scalar("1".to_owned()), true)
            .unwrap();
        assert!(
            matches!(scalar.value(), ShellValue::String(value) if value == &i64::MIN.to_string())
        );

        let mut indexed = ShellVariable::new(ShellValue::IndexedArray(BTreeMap::from([(
            0,
            i64::MAX.to_string(),
        )])));
        indexed.treat_as_integer();
        indexed
            .assign_at_index("0".to_owned(), "1".to_owned(), true)
            .unwrap();
        assert!(
            matches!(indexed.value(), ShellValue::IndexedArray(values) if values[&0] == i64::MIN.to_string())
        );

        let mut associative = ShellVariable::new(ShellValue::AssociativeArray(BTreeMap::from([(
            "key".to_owned(),
            i64::MAX.to_string(),
        )])));
        associative.treat_as_integer();
        associative
            .assign_at_index("key".to_owned(), "1".to_owned(), true)
            .unwrap();
        assert!(
            matches!(associative.value(), ShellValue::AssociativeArray(values) if values["key"] == i64::MIN.to_string())
        );
    }
}

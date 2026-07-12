//! Implements a shell variable environment.

use std::borrow::Cow;
use std::collections::hash_map;
use std::collections::{HashMap, HashSet};

use crate::engine::Shell;
use crate::engine::error;
use crate::engine::variables::{
    self, ShellValue, ShellValueLiteral, ShellValueUnsetType, ShellVariable,
};

const MAX_NAMEREF_DEPTH: usize = 128;

/// nameref 最终解析出的变量目标.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedVariableTarget {
    /// 目标变量名.
    pub name: String,
    /// 基础数组元素下标, `None` 表示整个变量.
    pub index: Option<String>,
}

/// Represents the policy for looking up variables in a shell environment.
#[derive(Clone, Copy)]
pub enum EnvironmentLookup {
    /// Look anywhere.
    Anywhere,
    /// Look only in the global scope.
    OnlyInGlobal,
    /// Look only in the current local scope.
    OnlyInCurrentLocal,
    /// Look only in local scopes.
    OnlyInLocal,
}

/// Represents a shell environment scope.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnvironmentScope {
    /// Scope local to a function instance
    Local,
    /// Globals
    Global,
    /// Transient overrides for a command invocation
    Command,
}

impl std::fmt::Display for EnvironmentScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Local => write!(f, "local"),
            Self::Global => write!(f, "global"),
            Self::Command => write!(f, "command"),
        }
    }
}

/// A guard that pushes a scope onto a shell environment and pops it when dropped.
pub(crate) struct ScopeGuard<'a> {
    scope_type: EnvironmentScope,
    shell: &'a mut crate::engine::Shell,
    detached: bool,
}

impl<'a> ScopeGuard<'a> {
    /// Creates a new scope guard, pushing the given scope type onto the environment.
    ///
    /// # Arguments
    ///
    /// * `shell` - The shell whose environment to modify.
    /// * `scope_type` - The type of scope to push.
    pub fn new(shell: &'a mut crate::engine::Shell, scope_type: EnvironmentScope) -> Self {
        shell.env_mut().push_scope(scope_type);
        Self {
            scope_type,
            shell,
            detached: false,
        }
    }

    /// Returns a mutable reference to the shell.
    pub const fn shell(&mut self) -> &mut crate::engine::Shell {
        self.shell
    }

    /// Detaches the guard, preventing it from popping the scope on drop.
    pub const fn detach(&mut self) {
        self.detached = true;
    }
}

impl Drop for ScopeGuard<'_> {
    fn drop(&mut self) {
        if !self.detached {
            let _ = self.shell.env_mut().pop_scope(self.scope_type);
        }
    }
}

/// Represents the shell variable environment, composed of a stack of scopes.
#[derive(Clone, Debug)]
pub struct ShellEnvironment {
    /// Stack of scopes, with the top of the stack being the current scope.
    scopes: Vec<(EnvironmentScope, ShellVariableMap)>,
    /// Whether or not to auto-export variables on creation or modification.
    export_variables_on_modification: bool,
    /// Count of total entries (may include duplicates with shadowed variables).
    entry_count: usize,
    /// 为无法通过旧式 `get_mut` API 表达的 nameref 错误提供可变错误目标.
    failed_nameref_assignment: ShellVariable,
}

impl Default for ShellEnvironment {
    fn default() -> Self {
        Self::new()
    }
}

impl ShellEnvironment {
    /// Returns a new shell environment.
    pub fn new() -> Self {
        Self {
            scopes: vec![(EnvironmentScope::Global, ShellVariableMap::default())],
            export_variables_on_modification: false,
            entry_count: 0,
            failed_nameref_assignment: ShellVariable::assignment_error(
                "assigning through a cyclic or unsupported nameref",
            ),
        }
    }

    /// Pushes a new scope of the given type onto the environment's scope stack.
    ///
    /// # Arguments
    ///
    /// * `scope_type` - The type of scope to push.
    pub fn push_scope(&mut self, scope_type: EnvironmentScope) {
        self.scopes.push((scope_type, ShellVariableMap::default()));
    }

    /// Pops the top-most scope off the environment's scope stack.
    ///
    /// # Arguments
    ///
    /// * `expected_scope_type` - The type of scope that is expected to be atop the stack.
    pub fn pop_scope(&mut self, expected_scope_type: EnvironmentScope) -> Result<(), error::Error> {
        // Root scope 始终保留, 并在修改栈之前验证作用域类型.
        if self.scopes.len() <= 1 {
            return Err(error::ErrorKind::MissingScope.into());
        }

        let (actual_scope_type, variables) =
            self.scopes.last().ok_or(error::ErrorKind::MissingScope)?;
        if *actual_scope_type != expected_scope_type {
            return Err(error::ErrorKind::UnexpectedScopeType {
                expected: expected_scope_type,
                actual: *actual_scope_type,
            }
            .into());
        }
        let next_entry_count = self
            .entry_count
            .checked_sub(variables.variables.len())
            .ok_or_else(|| {
                error::ErrorKind::InternalError("environment entry count underflow".to_owned())
            })?;

        self.scopes.pop().ok_or(error::ErrorKind::MissingScope)?;
        self.entry_count = next_entry_count;
        Ok(())
    }

    //
    // Iterators/Getters
    //

    /// Returns an iterator over all exported variables defined in the variable.
    pub fn iter_exported(&self) -> impl Iterator<Item = (&String, &ShellVariable)> {
        // We won't actually need to store all entries, but we expect it should be
        // within the same order.
        let mut visible_vars: HashMap<&String, Option<&ShellVariable>> =
            HashMap::with_capacity(self.entry_count);

        for (_, var_map) in self.scopes.iter().rev() {
            for (name, var) in var_map.iter() {
                // 先记录遮蔽关系, 再决定可见变量是否已导出.
                if let hash_map::Entry::Vacant(entry) = visible_vars.entry(name) {
                    entry.insert(var.is_exported().then_some(var));
                }
            }
        }

        visible_vars
            .into_iter()
            .filter_map(|(name, var)| var.map(|var| (name, var)))
    }

    /// Returns an iterator over all the variables defined in the environment.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &ShellVariable)> {
        self.iter_using_policy(EnvironmentLookup::Anywhere)
    }

    /// Returns an iterator over all the variables defined in the environment,
    /// using the given lookup policy.
    ///
    /// # Arguments
    ///
    /// * `lookup_policy` - The policy to use when looking up variables.
    pub fn iter_using_policy(
        &self,
        lookup_policy: EnvironmentLookup,
    ) -> impl Iterator<Item = (&String, &ShellVariable)> {
        // We won't actually need to store all entries, but we expect it should be
        // within the same order.
        let mut visible_vars: HashMap<&String, &ShellVariable> =
            HashMap::with_capacity(self.entry_count);

        let mut local_count = 0;
        for (scope_type, var_map) in self.scopes.iter().rev() {
            if matches!(scope_type, EnvironmentScope::Local) {
                local_count += 1;
            }

            match lookup_policy {
                EnvironmentLookup::Anywhere => (),
                EnvironmentLookup::OnlyInGlobal => {
                    if !matches!(scope_type, EnvironmentScope::Global) {
                        continue;
                    }
                }
                EnvironmentLookup::OnlyInCurrentLocal => {
                    if !(matches!(scope_type, EnvironmentScope::Local) && local_count == 1) {
                        continue;
                    }
                }
                EnvironmentLookup::OnlyInLocal => {
                    if !matches!(scope_type, EnvironmentScope::Local) {
                        continue;
                    }
                }
            }

            for (name, var) in var_map.iter() {
                // Only insert the variable if it hasn't been seen yet.
                if let hash_map::Entry::Vacant(entry) = visible_vars.entry(name) {
                    entry.insert(var);
                }
            }

            if matches!(scope_type, EnvironmentScope::Local)
                && matches!(lookup_policy, EnvironmentLookup::OnlyInCurrentLocal)
            {
                break;
            }
        }

        visible_vars.into_iter()
    }

    /// Tries to retrieve an immutable reference to the variable with the given name
    /// in the environment.
    ///
    /// # Arguments
    ///
    /// * `name` - The name of the variable to retrieve.
    pub fn get<S: AsRef<str>>(&self, name: S) -> Option<(EnvironmentScope, &ShellVariable)> {
        // Look through scopes, from the top of the stack on down.
        for (scope_type, map) in self.scopes.iter().rev() {
            if let Some(var) = map.get(name.as_ref()) {
                return Some((*scope_type, var));
            }
        }

        None
    }

    /// Tries to retrieve a mutable reference to the variable with the given name
    /// in the environment.
    ///
    /// # Arguments
    ///
    /// * `name` - The name of the variable to retrieve.
    pub fn get_mut<S: AsRef<str>>(
        &mut self,
        name: S,
    ) -> Option<(EnvironmentScope, &mut ShellVariable)> {
        let name = name.as_ref();
        let source_scope = self.get(name).map(|(scope, _)| scope)?;
        let target = match self.resolve_target(name) {
            Ok(target) if target.index.is_none() => target,
            Ok(_) | Err(_) => {
                return Some((source_scope, &mut self.failed_nameref_assignment));
            }
        };

        if self.get(target.name.as_str()).is_none()
            && self
                .add(
                    target.name.clone(),
                    ShellVariable::new(ShellValue::Unset(ShellValueUnsetType::Untyped)),
                    EnvironmentScope::Global,
                )
                .is_err()
        {
            return Some((source_scope, &mut self.failed_nameref_assignment));
        }

        self.get_raw_mut(target.name.as_str())
    }

    fn get_raw_mut(&mut self, name: &str) -> Option<(EnvironmentScope, &mut ShellVariable)> {
        for (scope_type, map) in self.scopes.iter_mut().rev() {
            if let Some(var) = map.get_mut(name) {
                return Some((*scope_type, var));
            }
        }

        None
    }

    /// 解析变量名所指向的最终 nameref 目标.
    pub fn resolve_target(&self, name: &str) -> Result<ResolvedVariableTarget, error::Error> {
        self.resolve_target_using_policy(name, EnvironmentLookup::Anywhere)
    }

    /// 按指定初始查询策略解析变量名所指向的最终 nameref 目标.
    pub fn resolve_target_using_policy(
        &self,
        name: &str,
        lookup_policy: EnvironmentLookup,
    ) -> Result<ResolvedVariableTarget, error::Error> {
        let mut current = name.to_owned();
        let mut seen = HashSet::new();

        for depth in 0..=MAX_NAMEREF_DEPTH {
            if !seen.insert(current.clone()) {
                return Err(error::ErrorKind::BadSubstitution(format!(
                    "nameref cycle involving '{current}'"
                ))
                .into());
            }
            if depth == MAX_NAMEREF_DEPTH {
                return Err(error::ErrorKind::BadSubstitution(format!(
                    "nameref maximum depth exceeded while resolving '{name}'"
                ))
                .into());
            }

            let variable = if depth == 0 {
                self.get_using_policy(current.as_str(), lookup_policy)
            } else {
                self.get(current.as_str()).map(|(_, variable)| variable)
            };
            let Some(variable) = variable else {
                return Ok(ResolvedVariableTarget {
                    name: current,
                    index: None,
                });
            };
            if !variable.is_treated_as_nameref() {
                return Ok(ResolvedVariableTarget {
                    name: current,
                    index: None,
                });
            }

            let ShellValue::String(target) = variable.value() else {
                if matches!(variable.value(), ShellValue::Unset(_)) {
                    return Ok(ResolvedVariableTarget {
                        name: current,
                        index: None,
                    });
                }
                return Err(error::ErrorKind::BadSubstitution(format!(
                    "nameref '{current}' does not contain a scalar target"
                ))
                .into());
            };
            let target = parse_nameref_target(target)?;
            if target.index.is_some() {
                return Ok(target);
            }
            current = target.name;
        }

        unreachable!("nameref depth loop always returns")
    }

    /// 返回已解析 nameref 的变量和可选基础数组下标.
    #[expect(clippy::type_complexity)]
    pub fn get_resolved(
        &self,
        name: &str,
    ) -> Result<Option<(EnvironmentScope, &ShellVariable, Option<String>)>, error::Error> {
        let target = self.resolve_target(name)?;
        Ok(self
            .get(target.name.as_str())
            .map(|(scope, variable)| (scope, variable, target.index)))
    }

    /// Tries to retrieve the string value of the variable with the given name in the
    /// environment.
    ///
    /// # Arguments
    ///
    /// * `name` - The name of the variable to retrieve.
    /// * `shell` - The shell owning the environment.
    pub fn get_str<S: AsRef<str>>(&self, name: S, shell: &Shell) -> Option<Cow<'_, str>> {
        let (_, variable, index) = self.get_resolved(name.as_ref()).ok()??;
        if let Some(index) = index {
            variable
                .value()
                .get_at(index.as_str(), shell)
                .ok()
                .flatten()
        } else {
            variable.value().try_get_cow_str(shell)
        }
    }

    /// Checks if a variable of the given name is set in the environment.
    ///
    /// # Arguments
    ///
    /// * `name` - The name of the variable to check.
    pub fn is_set<S: AsRef<str>>(&self, name: S) -> bool {
        if let Some((_, var)) = self.get(name) {
            !matches!(var.value(), ShellValue::Unset(_))
        } else {
            false
        }
    }

    //
    // Setters
    //

    /// Tries to unset the variable with the given name in the environment, returning
    /// whether or not such a variable existed.
    ///
    /// # Arguments
    ///
    /// * `name` - The name of the variable to unset.
    pub fn unset(&mut self, name: &str) -> Result<Option<ShellVariable>, error::Error> {
        let target = self.resolve_target(name)?;
        if let Some(index) = target.index {
            return self
                .unset_index_raw(target.name.as_str(), index.as_str())
                .map(|unset| unset.then(|| ShellVariable::new("")));
        }
        self.unset_raw(target.name.as_str())
    }

    /// 仅 unset 指定变量自身, 不跟随 nameref.
    pub fn unset_raw(&mut self, name: &str) -> Result<Option<ShellVariable>, error::Error> {
        let mut local_count = 0;
        for (scope_type, map) in self.scopes.iter_mut().rev() {
            if matches!(scope_type, EnvironmentScope::Local) {
                local_count += 1;
            }

            let unset_result = Self::try_unset_in_map(map, name)?;

            if unset_result.is_some() {
                // If we end up finding a local in the top-most local frame, then we replace
                // it with a placeholder.
                if matches!(scope_type, EnvironmentScope::Local) && local_count == 1 {
                    map.set(
                        name,
                        ShellVariable::new(ShellValue::Unset(ShellValueUnsetType::Untyped)),
                    );
                } else if self.entry_count > 0 {
                    // Entry count should never be 0 here, but we're being defensive.
                    self.entry_count -= 1;
                }

                return Ok(unset_result);
            }
        }

        Ok(None)
    }

    /// Tries to unset an array element from the environment, using the given name and
    /// element index for lookup. Returns whether or not an element was unset.
    ///
    /// # Arguments
    ///
    /// * `name` - The name of the array variable to unset an element from.
    /// * `index` - The index of the element to unset.
    pub fn unset_index(&mut self, name: &str, index: &str) -> Result<bool, error::Error> {
        let target = self.resolve_target(name)?;
        if target.index.is_some() {
            return Err(error::ErrorKind::BadSubstitution(
                "combining an explicit array index with an array-element nameref is unsupported"
                    .into(),
            )
            .into());
        }
        self.unset_index_raw(target.name.as_str(), index)
    }

    fn unset_index_raw(&mut self, name: &str, index: &str) -> Result<bool, error::Error> {
        if let Some((_, var)) = self.get_raw_mut(name) {
            var.unset_index(index)
        } else {
            Ok(false)
        }
    }

    fn try_unset_in_map(
        map: &mut ShellVariableMap,
        name: &str,
    ) -> Result<Option<ShellVariable>, error::Error> {
        match map.get(name).map(|v| v.is_readonly()) {
            Some(true) => Err(error::ErrorKind::ReadonlyVariable.into()),
            Some(false) => Ok(map.unset(name)),
            None => Ok(None),
        }
    }

    /// Tries to retrieve an immutable reference to a variable from the environment,
    /// using the given name and lookup policy.
    ///
    /// # Arguments
    ///
    /// * `name` - The name of the variable to retrieve.
    /// * `lookup_policy` - The policy to use when looking up the variable.
    pub fn get_using_policy<N: AsRef<str>>(
        &self,
        name: N,
        lookup_policy: EnvironmentLookup,
    ) -> Option<&ShellVariable> {
        let mut local_count = 0;
        for (scope_type, var_map) in self.scopes.iter().rev() {
            if matches!(scope_type, EnvironmentScope::Local) {
                local_count += 1;
            }

            match lookup_policy {
                EnvironmentLookup::Anywhere => (),
                EnvironmentLookup::OnlyInGlobal => {
                    if !matches!(scope_type, EnvironmentScope::Global) {
                        continue;
                    }
                }
                EnvironmentLookup::OnlyInCurrentLocal => {
                    if !(matches!(scope_type, EnvironmentScope::Local) && local_count == 1) {
                        continue;
                    }
                }
                EnvironmentLookup::OnlyInLocal => {
                    if !matches!(scope_type, EnvironmentScope::Local) {
                        continue;
                    }
                }
            }

            if let Some(var) = var_map.get(name.as_ref()) {
                return Some(var);
            }

            if matches!(scope_type, EnvironmentScope::Local)
                && matches!(lookup_policy, EnvironmentLookup::OnlyInCurrentLocal)
            {
                break;
            }
        }

        None
    }

    /// Tries to retrieve a mutable reference to a variable from the environment,
    /// using the given name and lookup policy.
    ///
    /// # Arguments
    ///
    /// * `name` - The name of the variable to retrieve.
    /// * `lookup_policy` - The policy to use when looking up the variable.
    pub fn get_mut_using_policy<N: AsRef<str>>(
        &mut self,
        name: N,
        lookup_policy: EnvironmentLookup,
    ) -> Option<&mut ShellVariable> {
        let mut local_count = 0;
        for (scope_type, var_map) in self.scopes.iter_mut().rev() {
            if matches!(scope_type, EnvironmentScope::Local) {
                local_count += 1;
            }

            match lookup_policy {
                EnvironmentLookup::Anywhere => (),
                EnvironmentLookup::OnlyInGlobal => {
                    if !matches!(scope_type, EnvironmentScope::Global) {
                        continue;
                    }
                }
                EnvironmentLookup::OnlyInCurrentLocal => {
                    if !(matches!(scope_type, EnvironmentScope::Local) && local_count == 1) {
                        continue;
                    }
                }
                EnvironmentLookup::OnlyInLocal => {
                    if !matches!(scope_type, EnvironmentScope::Local) {
                        continue;
                    }
                }
            }

            if let Some(var) = var_map.get_mut(name.as_ref()) {
                return Some(var);
            }

            if matches!(scope_type, EnvironmentScope::Local)
                && matches!(lookup_policy, EnvironmentLookup::OnlyInCurrentLocal)
            {
                break;
            }
        }

        None
    }

    /// Update a variable in the environment, or add it if it doesn't already exist.
    ///
    /// # Arguments
    ///
    /// * `name` - The name of the variable to update or add.
    /// * `value` - The value to assign to the variable.
    /// * `updater` - A function to call to update the variable after assigning the value.
    /// * `lookup_policy` - The policy to use when looking up the variable.
    /// * `scope_if_creating` - The scope to create the variable in if it doesn't already exist.
    pub fn update_or_add<N: Into<String>>(
        &mut self,
        name: N,
        value: variables::ShellValueLiteral,
        updater: impl Fn(&mut ShellVariable) -> Result<(), error::Error>,
        lookup_policy: EnvironmentLookup,
        scope_if_creating: EnvironmentScope,
    ) -> Result<(), error::Error> {
        let name = name.into();
        let target = self.resolve_target_using_policy(&name, lookup_policy)?;
        if let Some(index) = target.index {
            let ShellValueLiteral::Scalar(value) = value else {
                return Err(error::ErrorKind::AssigningListToArrayMember.into());
            };
            return self.update_or_add_array_element_raw(
                target.name,
                index,
                value,
                updater,
                EnvironmentLookup::Anywhere,
                scope_if_creating,
            );
        }

        let name = target.name;
        let auto_export = self.export_variables_on_modification;
        if let Some(var) = self.get_mut_using_policy(&name, EnvironmentLookup::Anywhere) {
            var.assign(value, false)?;
            if auto_export {
                var.export();
            }
            updater(var)
        } else {
            let mut var = ShellVariable::new(ShellValue::Unset(ShellValueUnsetType::Untyped));
            var.assign(value, false)?;
            if auto_export {
                var.export();
            }
            updater(&mut var)?;

            self.add(name, var, scope_if_creating)
        }
    }

    /// Update an array element in the environment, or add it if it doesn't already exist.
    ///
    /// # Arguments
    ///
    /// * `name` - The name of the variable to update or add.
    /// * `index` - The index of the element to update or add.
    /// * `value` - The value to assign to the variable.
    /// * `updater` - A function to call to update the variable after assigning the value.
    /// * `lookup_policy` - The policy to use when looking up the variable.
    /// * `scope_if_creating` - The scope to create the variable in if it doesn't already exist.
    pub fn update_or_add_array_element<N: Into<String>>(
        &mut self,
        name: N,
        index: String,
        value: String,
        updater: impl Fn(&mut ShellVariable) -> Result<(), error::Error>,
        lookup_policy: EnvironmentLookup,
        scope_if_creating: EnvironmentScope,
    ) -> Result<(), error::Error> {
        let name = name.into();
        let target = self.resolve_target_using_policy(&name, lookup_policy)?;
        if target.index.is_some() {
            return Err(error::ErrorKind::BadSubstitution(
                "combining an explicit array index with an array-element nameref is unsupported"
                    .into(),
            )
            .into());
        }
        self.update_or_add_array_element_raw(
            target.name,
            index,
            value,
            updater,
            EnvironmentLookup::Anywhere,
            scope_if_creating,
        )
    }

    fn update_or_add_array_element_raw(
        &mut self,
        name: String,
        index: String,
        value: String,
        updater: impl Fn(&mut ShellVariable) -> Result<(), error::Error>,
        lookup_policy: EnvironmentLookup,
        scope_if_creating: EnvironmentScope,
    ) -> Result<(), error::Error> {
        if let Some(var) = self.get_mut_using_policy(&name, lookup_policy) {
            var.assign_at_index(index, value, false)?;
            updater(var)
        } else {
            let mut var = ShellVariable::new(ShellValue::Unset(ShellValueUnsetType::Untyped));
            var.assign(
                variables::ShellValueLiteral::Array(variables::ArrayLiteral(vec![(
                    Some(index),
                    value,
                )])),
                false,
            )?;
            updater(&mut var)?;

            self.add(name, var, scope_if_creating)
        }
    }

    /// Adds a variable to the environment.
    ///
    /// # Arguments
    ///
    /// * `name` - The name of the variable to add.
    /// * `var` - The variable to add.
    /// * `target_scope` - The scope to add the variable to.
    pub fn add<N: Into<String>>(
        &mut self,
        name: N,
        mut var: ShellVariable,
        target_scope: EnvironmentScope,
    ) -> Result<(), error::Error> {
        if self.export_variables_on_modification {
            var.export();
        }

        for (scope_type, map) in self.scopes.iter_mut().rev() {
            if *scope_type == target_scope {
                let name = name.into();
                let next_entry_count = if map.get(name.as_str()).is_none() {
                    self.entry_count.checked_add(1).ok_or_else(|| {
                        error::ErrorKind::InternalError(
                            "environment entry count overflow".to_owned(),
                        )
                    })?
                } else {
                    self.entry_count
                };

                map.set(name, var);
                self.entry_count = next_entry_count;
                return Ok(());
            }
        }

        Err(error::ErrorKind::MissingScopeForNewVariable.into())
    }

    /// Sets a global variable in the environment.
    ///
    /// # Arguments
    ///
    /// * `name` - The name of the variable to set.
    /// * `var` - The variable to set.
    pub fn set_global<N: Into<String>>(
        &mut self,
        name: N,
        var: ShellVariable,
    ) -> Result<(), error::Error> {
        self.add(name, var, EnvironmentScope::Global)
    }
}

/// Represents a map from names to shell variables.
#[derive(Clone, Debug, Default)]
pub struct ShellVariableMap {
    variables: HashMap<String, ShellVariable>,
}

impl ShellVariableMap {
    //
    // Iterators/Getters
    //

    /// Returns an iterator over all the variables in the map.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &ShellVariable)> {
        self.variables.iter()
    }

    /// Tries to retrieve an immutable reference to the variable with the given name.
    ///
    /// # Arguments
    ///
    /// * `name` - The name of the variable to retrieve.
    pub fn get(&self, name: &str) -> Option<&ShellVariable> {
        self.variables.get(name)
    }

    /// Tries to retrieve a mutable reference to the variable with the given name.
    ///
    /// # Arguments
    ///
    /// * `name` - The name of the variable to retrieve.
    pub fn get_mut(&mut self, name: &str) -> Option<&mut ShellVariable> {
        self.variables.get_mut(name)
    }

    //
    // Setters
    //

    /// Tries to unset the variable with the given name, returning the removed
    /// variable or None if it was not already set.
    ///
    /// # Arguments
    ///
    /// * `name` - The name of the variable to unset.
    pub fn unset(&mut self, name: &str) -> Option<ShellVariable> {
        self.variables.remove(name)
    }

    /// Sets a variable in the map.
    ///
    /// # Arguments
    ///
    /// * `name` - The name of the variable to set.
    /// * `var` - The variable to set.
    pub fn set<N: Into<String>>(&mut self, name: N, var: ShellVariable) -> Option<ShellVariable> {
        self.variables.insert(name.into(), var)
    }
}

/// Checks if the given name is a valid variable name.
fn parse_nameref_target(target: &str) -> Result<ResolvedVariableTarget, error::Error> {
    if valid_variable_name(target) {
        return Ok(ResolvedVariableTarget {
            name: target.to_owned(),
            index: None,
        });
    }

    if let Some(without_closing) = target.strip_suffix(']')
        && let Some(opening) = without_closing.find('[')
    {
        let name = &without_closing[..opening];
        let index = &without_closing[opening + 1..];
        if valid_variable_name(name) && !index.is_empty() && !index.contains(['[', ']']) {
            return Ok(ResolvedVariableTarget {
                name: name.to_owned(),
                index: Some(index.to_owned()),
            });
        }
    }

    Err(error::ErrorKind::BadSubstitution(format!("invalid nameref target '{target}'")).into())
}

pub fn valid_variable_name(s: &str) -> bool {
    let mut cs = s.chars();
    match cs.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {
            cs.all(|c| c.is_ascii_alphanumeric() || c == '_')
        }
        Some(_) | None => false,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn pop_scope_validates_before_mutating_and_preserves_root() {
        let mut env = ShellEnvironment::new();
        env.set_global("global", ShellVariable::new("root"))
            .unwrap();
        env.push_scope(EnvironmentScope::Local);
        env.add(
            "local",
            ShellVariable::new("value"),
            EnvironmentScope::Local,
        )
        .unwrap();

        assert_eq!(env.entry_count, 2);
        assert!(env.pop_scope(EnvironmentScope::Command).is_err());
        assert_eq!(env.scopes.len(), 2);
        assert_eq!(env.entry_count, 2);
        assert!(env.get("local").is_some());

        env.pop_scope(EnvironmentScope::Local).unwrap();
        assert_eq!(env.scopes.len(), 1);
        assert_eq!(env.entry_count, 1);
        assert!(env.get("local").is_none());
        assert!(env.get("global").is_some());

        assert!(env.pop_scope(EnvironmentScope::Global).is_err());
        assert_eq!(env.scopes.len(), 1);
        assert_eq!(env.entry_count, 1);
        assert!(env.get("global").is_some());
    }

    #[test]
    fn iter_exported_respects_unexported_shadowing_variables() {
        let mut env = ShellEnvironment::new();
        let mut global = ShellVariable::new("global");
        global.export();
        env.set_global("shadowed", global).unwrap();
        let mut visible = ShellVariable::new("visible");
        visible.export();
        env.set_global("visible", visible).unwrap();

        env.push_scope(EnvironmentScope::Local);
        env.add(
            "shadowed",
            ShellVariable::new("local"),
            EnvironmentScope::Local,
        )
        .unwrap();

        let exported_names: Vec<_> = env.iter_exported().map(|(name, _)| name.as_str()).collect();
        assert!(!exported_names.contains(&"shadowed"));
        assert!(exported_names.contains(&"visible"));
    }

    fn nameref(target: &str) -> ShellVariable {
        let mut variable = ShellVariable::new(target);
        variable.treat_as_nameref();
        variable
    }

    #[test]
    fn nameref_resolution_handles_scope_missing_targets_and_array_elements() {
        let mut env = ShellEnvironment::new();
        env.set_global("value", ShellVariable::new("global"))
            .unwrap();
        env.set_global("ref", nameref("value")).unwrap();
        assert_eq!(env.resolve_target("ref").unwrap().name, "value");

        env.push_scope(EnvironmentScope::Local);
        env.add(
            "value",
            ShellVariable::new("local"),
            EnvironmentScope::Local,
        )
        .unwrap();
        let (_, variable, _) = env.get_resolved("ref").unwrap().unwrap();
        assert!(matches!(variable.value(), ShellValue::String(value) if value == "local"));

        env.set_global("missing_ref", nameref("created_later"))
            .unwrap();
        assert_eq!(
            env.resolve_target("missing_ref").unwrap(),
            ResolvedVariableTarget {
                name: "created_later".into(),
                index: None,
            }
        );

        env.set_global("element_ref", nameref("items[2]")).unwrap();
        assert_eq!(
            env.resolve_target("element_ref").unwrap(),
            ResolvedVariableTarget {
                name: "items".into(),
                index: Some("2".into()),
            }
        );
    }

    #[test]
    fn nameref_resolution_rejects_cycles_and_excessive_depth() {
        let mut env = ShellEnvironment::new();
        env.set_global("a", nameref("b")).unwrap();
        env.set_global("b", nameref("a")).unwrap();
        assert!(
            env.resolve_target("a")
                .unwrap_err()
                .to_string()
                .contains("cycle")
        );

        let mut deep = ShellEnvironment::new();
        for index in 0..=MAX_NAMEREF_DEPTH {
            deep.set_global(
                format!("ref{index}"),
                nameref(format!("ref{}", index + 1).as_str()),
            )
            .unwrap();
        }
        assert!(
            deep.resolve_target("ref0")
                .unwrap_err()
                .to_string()
                .contains("maximum depth")
        );
    }

    #[test]
    fn mutable_assignment_and_unset_follow_nameref_targets() {
        let mut env = ShellEnvironment::new();
        env.set_global("target", ShellVariable::new("before"))
            .unwrap();
        env.set_global("ref", nameref("target")).unwrap();

        env.get_mut("ref")
            .unwrap()
            .1
            .assign(ShellValueLiteral::Scalar("after".into()), false)
            .unwrap();
        assert!(
            matches!(env.get("target").unwrap().1.value(), ShellValue::String(value) if value == "after")
        );

        env.unset("ref").unwrap();
        assert!(env.get("target").is_none());
        assert!(env.get("ref").is_some());
        env.unset_raw("ref").unwrap();
        assert!(env.get("ref").is_none());
    }

    #[test]
    fn nameref_assignment_honors_readonly_target_and_rejects_cycles() {
        let mut env = ShellEnvironment::new();
        let mut target = ShellVariable::new("value");
        target.set_readonly();
        env.set_global("target", target).unwrap();
        env.set_global("ref", nameref("target")).unwrap();
        let error = env
            .get_mut("ref")
            .unwrap()
            .1
            .assign(ShellValueLiteral::Scalar("new".into()), false)
            .unwrap_err();
        assert!(matches!(error.kind(), error::ErrorKind::ReadonlyVariable));

        env.set_global("a", nameref("b")).unwrap();
        env.set_global("b", nameref("a")).unwrap();
        assert!(
            env.get_mut("a")
                .unwrap()
                .1
                .assign(ShellValueLiteral::Scalar("new".into()), false)
                .unwrap_err()
                .to_string()
                .contains("cyclic or unsupported nameref")
        );
    }

    #[test]
    fn test_valid_variable_name() {
        assert!(!valid_variable_name(""));
        assert!(!valid_variable_name("1"));
        assert!(!valid_variable_name(" a"));
        assert!(!valid_variable_name(" "));

        assert!(valid_variable_name("_"));
        assert!(valid_variable_name("_a"));
        assert!(valid_variable_name("_1"));
        assert!(valid_variable_name("_a1"));
        assert!(valid_variable_name("a"));
        assert!(valid_variable_name("A"));
        assert!(valid_variable_name("a1"));
        assert!(valid_variable_name("A1"));
    }
}

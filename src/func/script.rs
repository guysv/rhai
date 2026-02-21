//! Implement script function-calling mechanism for [`Engine`].
#![cfg(not(feature = "no_function"))]

use super::call::FnCallArgs;
use crate::ast::{EncapsulatedEnviron, ScriptFuncDef};
use crate::eval::{Caches, GlobalRuntimeState};
use crate::{
    BorrowedScopeEntry, BorrowedScopeValue, Dynamic, Engine, Position, RhaiResult, Scope, ERR,
};
#[cfg(feature = "no_std")]
use std::prelude::v1::*;
use std::mem;

impl Engine {
    /// # Main Entry-Point
    ///
    /// Call a script-defined function.
    ///
    /// If `rewind_scope` is `false`, arguments are removed from the scope but new variables are not.
    ///
    /// # WARNING
    ///
    /// Function call arguments may be _consumed_ when the function requires them to be passed by value.
    /// All function arguments not in the first position are always passed by value and thus consumed.
    ///
    /// **DO NOT** reuse the argument values except for the first `&mut` argument - all others are silently replaced by `()`!
    pub(crate) fn call_script_fn(
        &self,
        global: &mut GlobalRuntimeState,
        caches: &mut Caches,
        scope: &mut Scope,
        mut this_ptr: Option<&mut Dynamic>,
        _env: Option<&EncapsulatedEnviron>,
        fn_def: &ScriptFuncDef,
        args: &mut FnCallArgs,
        mut borrowed_scope: Option<&mut [BorrowedScopeEntry<'_>]>,
        rewind_scope: bool,
        pos: Position,
    ) -> RhaiResult {
        debug_assert_eq!(fn_def.params.len(), args.len());

        self.track_operation(global, pos)?;

        // Check for stack overflow
        #[cfg(not(feature = "unchecked"))]
        if global.level > self.max_call_levels() {
            return Err(ERR::ErrorStackOverflow(pos).into());
        }

        #[cfg(feature = "debugging")]
        if self.debugger_interface.is_none() && fn_def.body.is_empty() {
            return Ok(Dynamic::UNIT);
        }
        #[cfg(not(feature = "debugging"))]
        if fn_def.body.is_empty() {
            return Ok(Dynamic::UNIT);
        }

        let orig_scope_len = scope.len();
        let orig_lib_len = global.lib.len();
        #[cfg(not(feature = "no_module"))]
        let orig_imports_len = global.num_imports();

        #[cfg(feature = "debugging")]
        let orig_call_stack_len = global
            .debugger
            .as_ref()
            .map_or(0, |dbg| dbg.call_stack().len());

        #[cfg(not(feature = "no_closure"))]
        let borrowed_scope_count = borrowed_scope.as_ref().map_or(0, |bindings| {
            bindings
                .iter()
                .filter(|binding| matches!(binding.value, BorrowedScopeValue::Dynamic(..)))
                .count()
        });
        #[cfg(feature = "no_closure")]
        let borrowed_scope_count = 0;

        // Guard against too many variables
        #[cfg(not(feature = "unchecked"))]
        if scope.len() + fn_def.params.len() + borrowed_scope_count > self.max_variables() {
            return Err(ERR::ErrorTooManyVariables(pos).into());
        }

        #[cfg(not(feature = "no_closure"))]
        let mut borrowed_scope_values = crate::FnArgsVec::new_const();
        #[cfg(not(feature = "no_closure"))]
        let mut borrowed_scope_map = crate::func::native::ActiveBorrowedBindings::new();
        let mut orig_borrowed_scope = None;

        #[cfg(feature = "no_closure")]
        if borrowed_scope.as_ref().map_or(false, |bindings| !bindings.is_empty()) {
            return Err(ERR::ErrorRuntime(
                "borrowed scope bindings require the `no_closure` feature to be disabled".into(),
                pos,
            )
            .into());
        }

        #[cfg(not(feature = "no_closure"))]
        if let Some(bindings) = borrowed_scope.as_deref_mut() {
            for binding in bindings.iter() {
                if matches!(binding.value, BorrowedScopeValue::Dynamic(..))
                    && (fn_def.params.iter().any(|param| param == &binding.name)
                        || scope.contains(binding.name.as_str()))
                {
                    return Err(ERR::ErrorRuntime(
                        format!(
                            "borrowed scope binding '{}' conflicts with an existing variable",
                            binding.name
                        )
                        .into(),
                        pos,
                    )
                    .into());
                }
            }

            borrowed_scope_values.reserve(bindings.len());

            for binding in bindings.iter_mut() {
                match &mut binding.value {
                    BorrowedScopeValue::Dynamic(value) => {
                        let value = mem::take(*value).into_shared();
                        scope.push_dynamic(binding.name.clone(), value.clone());
                        borrowed_scope_map.insert(
                            binding.name.clone(),
                            crate::func::native::ActiveBorrowedValue::Dynamic(value.clone()),
                        );
                        borrowed_scope_values.push(value);
                    }
                    BorrowedScopeValue::Opaque(ptr) => {
                        borrowed_scope_map.insert(
                            binding.name.clone(),
                            crate::func::native::ActiveBorrowedValue::Opaque(ptr.ptr),
                        );
                    }
                }
            }

            if !borrowed_scope_map.is_empty() {
                let map = crate::Shared::new(crate::Locked::new(borrowed_scope_map));
                orig_borrowed_scope = Some(std::mem::replace(
                    &mut global.active_borrowed_scope,
                    Some(map),
                ));
            }
        }

        defer! {
            global if orig_borrowed_scope.is_some() =>
            move |g| g.active_borrowed_scope = orig_borrowed_scope.unwrap()
        }

        // Put arguments into scope as variables.
        // Keep function parameters as the newest entries so pre-computed variable offsets remain valid.
        scope.extend(fn_def.params.iter().cloned().zip(args.iter_mut().map(|v| {
            // Actually consume the arguments instead of cloning them
            v.take()
        })));

        // Push a new call stack frame
        #[cfg(feature = "debugging")]
        if self.is_debugger_registered() {
            let fn_name = fn_def.name.clone();
            let args = scope
                .iter_inner()
                .skip(orig_scope_len)
                .map(|(.., v)| v.flatten_clone());
            let source = global.source.clone();

            global
                .debugger_mut()
                .push_call_stack_frame(fn_name, args, source, pos);
        }

        // Merge in encapsulated environment, if any
        let orig_fn_resolution_caches_len = caches.fn_resolution_caches_len();

        #[cfg(not(feature = "no_module"))]
        let orig_constants = _env.map(
            |EncapsulatedEnviron {
                 lib,
                 imports,
                 constants,
             }| {
                imports
                    .iter()
                    .cloned()
                    .for_each(|(n, m)| global.push_import(n, m));

                global.lib.extend(lib.clone());

                std::mem::replace(&mut global.constants, constants.clone())
            },
        );

        #[cfg(feature = "debugging")]
        if self.is_debugger_registered() {
            let node = crate::ast::Stmt::Noop(fn_def.body.position());
            self.dbg(global, caches, scope, this_ptr.as_deref_mut(), &node)?;
        }

        // Evaluate the function
        let mut _result: RhaiResult = self
            .eval_stmt_block(
                global,
                caches,
                scope,
                this_ptr.as_deref_mut(),
                fn_def.body.statements(),
                rewind_scope,
            )
            .or_else(|err| match *err {
                // Convert return statement to return value
                ERR::Return(x, ..) => Ok(x),
                // Exit value is passed straight-through
                mut err @ ERR::Exit(..) => {
                    err.set_position(pos);
                    Err(err.into())
                }
                // System errors are passed straight-through
                mut err if err.is_system_exception() => {
                    err.set_position(pos);
                    Err(err.into())
                }
                // Other errors are wrapped in `ErrorInFunctionCall`
                _ => Err(ERR::ErrorInFunctionCall(
                    fn_def.name.to_string(),
                    #[cfg(not(feature = "no_module"))]
                    _env.and_then(|env| env.lib.last())
                        .and_then(|m| m.id())
                        .unwrap_or_else(|| global.source().unwrap_or(""))
                        .to_string(),
                    #[cfg(feature = "no_module")]
                    global.source().unwrap_or("").to_string(),
                    err,
                    pos,
                )
                .into()),
            });

        #[cfg(feature = "debugging")]
        if self.is_debugger_registered() {
            let trigger = match global.debugger_mut().status {
                crate::eval::DebuggerStatus::FunctionExit(n) => n >= global.level,
                crate::eval::DebuggerStatus::Next(.., true) => true,
                _ => false,
            };

            if trigger {
                let node = crate::ast::Stmt::Noop(fn_def.body.end_position().or_else(pos));
                let node = (&node).into();
                let event = match _result {
                    Ok(ref r) => crate::eval::DebuggerEvent::FunctionExitWithValue(r),
                    Err(ref err) => crate::eval::DebuggerEvent::FunctionExitWithError(err),
                };
                match self.dbg_raw(global, caches, scope, this_ptr, node, event) {
                    Ok(_) => (),
                    Err(err) => _result = Err(err),
                }
            }

            // Pop the call stack
            global
                .debugger
                .as_mut()
                .unwrap()
                .rewind_call_stack(orig_call_stack_len);
        }

        // Remove all local variables and imported modules
        #[cfg(not(feature = "no_closure"))]
        if let Some(bindings) = borrowed_scope.as_deref_mut() {
            let mut index = 0;

            for binding in bindings.iter_mut() {
                if let BorrowedScopeValue::Dynamic(value) = &mut binding.value {
                    let shared_value = &mut borrowed_scope_values[index];
                    index += 1;

                    let new_value = shared_value
                        .read_lock::<Dynamic>()
                        .map_or_else(|| shared_value.flatten_clone(), |v| v.flatten_clone());
                    **value = new_value;

                    if let Some(mut guard) = shared_value.write_lock::<Dynamic>() {
                        *guard = Dynamic::make_expired_borrowed_binding(binding.name.clone());
                    }
                }
            }
        }

        #[cfg(not(feature = "no_closure"))]
        let num_bound_values = args.len() + borrowed_scope_values.len();
        #[cfg(feature = "no_closure")]
        let num_bound_values = args.len();

        if rewind_scope {
            scope.rewind(orig_scope_len);
        } else if num_bound_values > 0 {
            // Remove arguments and borrowed bindings only, leaving new variables in the scope
            scope.remove_range(orig_scope_len, num_bound_values);
        }
        global.lib.truncate(orig_lib_len);
        #[cfg(not(feature = "no_module"))]
        global.truncate_imports(orig_imports_len);

        // Restore constants
        #[cfg(not(feature = "no_module"))]
        if let Some(constants) = orig_constants {
            global.constants = constants;
        }

        // Restore state
        caches.rewind_fn_resolution_caches(orig_fn_resolution_caches_len);

        _result
    }

    // Does a script-defined function exist?
    ///
    /// # Note
    ///
    /// If the scripted function is not found, this information is cached for future look-ups.
    #[must_use]
    pub(crate) fn has_script_fn(
        &self,
        global: &GlobalRuntimeState,
        caches: &mut Caches,
        hash_script: u64,
    ) -> bool {
        let cache = caches.fn_resolution_cache_mut();

        if let Some(result) = cache.dict.get(&hash_script).map(Option::is_some) {
            return result;
        }

        // First check script-defined functions
        let res = global.lib.iter().any(|m| m.contains_fn(hash_script))
            // Then check the global namespace and packages
            || self.global_modules.iter().any(|m| m.contains_fn(hash_script));

        #[cfg(not(feature = "no_module"))]
        let res = res ||
            // Then check imported modules
            global.contains_qualified_fn(hash_script)
            // Then check sub-modules
            || self.global_sub_modules.values().any(|m| m.contains_qualified_fn(hash_script));

        if !res && !cache.bloom_filter.is_absent_and_set(hash_script) {
            // Do not cache "one-hit wonders"
            cache.dict.insert(hash_script, None);
        }

        res
    }
}

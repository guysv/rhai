//! Helper module which defines [`FuncArgs`] to make function calling easier.

#![allow(non_snake_case)]

use crate::types::dynamic::Variant;
use crate::{Dynamic, ImmutableString};
use std::any::Any;
use std::marker::PhantomData;
use std::ptr::NonNull;
#[cfg(feature = "no_std")]
use std::prelude::v1::*;

/// Trait that parses arguments to a function call.
///
/// Any data type can implement this trait in order to pass arguments to
/// [`Engine::call_fn`][crate::Engine::call_fn].
pub trait FuncArgs {
    /// Parse function call arguments into a container.
    ///
    /// # Example
    ///
    /// ```
    /// use rhai::{Engine, Dynamic, FuncArgs, Scope};
    ///
    /// // A struct containing function arguments
    /// struct Options {
    ///     pub foo: bool,
    ///     pub bar: String,
    ///     pub baz: i64,
    /// }
    ///
    /// impl FuncArgs for Options {
    ///     fn parse<ARGS: Extend<Dynamic>>(self, args: &mut ARGS) {
    ///         args.extend(Some(self.foo.into()));
    ///         args.extend(Some(self.bar.into()));
    ///         args.extend(Some(self.baz.into()));
    ///     }
    /// }
    ///
    /// # fn main() -> Result<(), Box<rhai::EvalAltResult>> {
    /// # #[cfg(not(feature = "no_function"))]
    /// # {
    /// let options = Options { foo: false, bar: "world".to_string(), baz: 42 };
    ///
    /// let engine = Engine::new();
    /// let mut scope = Scope::new();
    ///
    /// let ast = engine.compile(
    /// "
    ///     fn hello(x, y, z) {
    ///         if x { `hello ${y}` } else { y + z }
    ///     }
    /// ")?;
    ///
    /// let result: String = engine.call_fn(&mut scope, &ast, "hello", options)?;
    ///
    /// assert_eq!(result, "world42");
    /// # }
    /// # Ok(())
    /// # }
    /// ```
    fn parse<ARGS: Extend<Dynamic>>(self, args: &mut ARGS);
}

/// A borrowed scope binding for [`Engine::call_fn_with_borrowed_scope`][crate::Engine::call_fn_with_borrowed_scope].
#[derive(Debug)]
pub struct BorrowedScopeEntry<'a> {
    /// The binding name.
    pub(crate) name: ImmutableString,
    /// Borrowed binding value.
    pub(crate) value: BorrowedScopeValue<'a>,
}

/// A script-visible token that references an active borrowed binding by name.
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct BorrowToken {
    name: ImmutableString,
}

impl BorrowToken {
    /// Create a new borrowed-binding token.
    #[inline(always)]
    #[must_use]
    pub fn new(name: impl Into<ImmutableString>) -> Self {
        Self { name: name.into() }
    }
    /// Get the borrowed binding name carried by this token.
    #[inline(always)]
    #[must_use]
    pub fn name(&self) -> &str {
        self.name.as_str()
    }
}

/// Value of a borrowed scope binding.
#[derive(Debug)]
pub(crate) enum BorrowedScopeValue<'a> {
    /// Borrowed [`Dynamic`] value exposed as a script local.
    Dynamic(&'a mut Dynamic),
    /// Borrowed opaque Rust value exposed only to `register_borrow_fn` handlers.
    Opaque(BorrowedOpaquePointer<'a>),
}

/// Opaque borrowed pointer to any Rust value.
#[derive(Debug)]
pub(crate) struct BorrowedOpaquePointer<'a> {
    pub(crate) ptr: NonNull<dyn Any>,
    _marker: PhantomData<&'a mut dyn Any>,
}

impl<'a> BorrowedOpaquePointer<'a> {
    /// Create an opaque pointer from a mutable Rust value.
    #[inline(always)]
    #[must_use]
    pub fn new<T: Any>(value: &'a mut T) -> Self {
        Self {
            ptr: NonNull::from(value as &mut dyn Any),
            _marker: PhantomData,
        }
    }
}

impl<'a> BorrowedScopeEntry<'a> {
    /// Create a borrowed binding from a [`Dynamic`] value.
    #[inline(always)]
    #[must_use]
    pub fn dynamic(name: impl Into<ImmutableString>, value: &'a mut Dynamic) -> Self {
        Self {
            name: name.into(),
            value: BorrowedScopeValue::Dynamic(value),
        }
    }
    /// Create a borrowed binding from an opaque Rust value.
    #[inline(always)]
    #[must_use]
    pub fn opaque<T: Any>(name: impl Into<ImmutableString>, value: &'a mut T) -> Self {
        Self {
            name: name.into(),
            value: BorrowedScopeValue::Opaque(BorrowedOpaquePointer::new(value)),
        }
    }
}

/// Trait that parses borrowed scope bindings for a function call.
///
/// Any data type can implement this trait in order to pass borrowed scope bindings to
/// [`Engine::call_fn_with_borrowed_scope`][crate::Engine::call_fn_with_borrowed_scope].
///
/// For most use cases, prefer passing an array or `Vec<BorrowedScopeEntry>` built with:
///
/// - [`BorrowedScopeEntry::dynamic`] for borrowed [`Dynamic`] values used directly by script
/// - [`BorrowedScopeEntry::opaque`] for opaque non-`Clone` Rust values used by `register_borrow_fn`
pub trait BorrowedFuncArgs<'a> {
    /// Parse borrowed bindings into a container.
    fn parse<ARGS: Extend<BorrowedScopeEntry<'a>>>(self, args: &mut ARGS);
}

impl<'a> BorrowedFuncArgs<'a> for () {
    #[inline(always)]
    fn parse<ARGS: Extend<BorrowedScopeEntry<'a>>>(self, _: &mut ARGS) {}
}

impl<'a> BorrowedFuncArgs<'a> for Vec<BorrowedScopeEntry<'a>> {
    #[inline]
    fn parse<ARGS: Extend<BorrowedScopeEntry<'a>>>(self, args: &mut ARGS) {
        args.extend(self);
    }
}

impl<'a, const LEN: usize> BorrowedFuncArgs<'a> for [BorrowedScopeEntry<'a>; LEN] {
    #[inline]
    fn parse<ARGS: Extend<BorrowedScopeEntry<'a>>>(self, args: &mut ARGS) {
        args.extend(IntoIterator::into_iter(self));
    }
}

impl<T: Variant + Clone> FuncArgs for Vec<T> {
    #[inline]
    fn parse<ARGS: Extend<Dynamic>>(self, args: &mut ARGS) {
        args.extend(self.into_iter().map(Dynamic::from));
    }
}

impl<T: Variant + Clone, const N: usize> FuncArgs for [T; N] {
    #[inline]
    fn parse<ARGS: Extend<Dynamic>>(self, args: &mut ARGS) {
        args.extend(IntoIterator::into_iter(self).map(Dynamic::from));
    }
}

/// Macro to implement [`FuncArgs`] for tuples of standard types (each can be converted into a [`Dynamic`]).
macro_rules! impl_args {
    ($($p:ident),*) => {
        impl<$($p: Variant + Clone),*> FuncArgs for ($($p,)*)
        {
            #[inline]
            #[allow(unused_variables)]
            fn parse<ARGS: Extend<Dynamic>>(self, args: &mut ARGS) {
                let ($($p,)*) = self;
                $(args.extend(Some(Dynamic::from($p)));)*
            }
        }

        impl_args!(@pop $($p),*);
    };
    (@pop) => {
    };
    (@pop $head:ident) => {
        impl_args!();
    };
    (@pop $head:ident $(, $tail:ident)+) => {
        impl_args!($($tail),*);
    };
}

impl_args!(A, B, C, D, E, F, G, H, J, K, L, M, N, P, Q, R, S, T, U, V);

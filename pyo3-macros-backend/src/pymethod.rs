// Copyright (c) 2017-present PyO3 Project and Contributors

use std::borrow::Cow;

use crate::attributes::NameAttribute;
use crate::method::ExtractErrorMode;
use crate::utils::{ensure_not_async_fn, unwrap_ty_group, PythonDoc};
use crate::{deprecations::Deprecations, utils};
use crate::{
    method::{FnArg, FnSpec, FnType, SelfType},
    pyfunction::PyFunctionOptions,
};
use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote, ToTokens};
use syn::Ident;
use syn::{ext::IdentExt, spanned::Spanned, Result};

pub enum GeneratedPyMethod {
    Method(TokenStream),
    Proto(TokenStream),
    TraitImpl(TokenStream),
    SlotTraitImpl(String, TokenStream),
}

pub fn gen_py_method(
    cls: &syn::Type,
    sig: &mut syn::Signature,
    meth_attrs: &mut Vec<syn::Attribute>,
    options: PyFunctionOptions,
) -> Result<GeneratedPyMethod> {
    check_generic(sig)?;
    ensure_not_async_fn(sig)?;
    ensure_function_options_valid(&options)?;
    let spec = FnSpec::parse(sig, &mut *meth_attrs, options)?;

    let method_name = spec.python_name.to_string();

    if let Some(slot_def) = pyproto(&method_name) {
        let slot = slot_def.generate_type_slot(cls, &spec)?;
        return Ok(GeneratedPyMethod::Proto(slot));
    }

    if let Some(slot_fragment_def) = pyproto_fragment(&method_name) {
        let proto = slot_fragment_def.generate_pyproto_fragment(cls, &spec)?;
        return Ok(GeneratedPyMethod::SlotTraitImpl(method_name, proto));
    }

    Ok(match &spec.tp {
        // ordinary functions (with some specialties)
        FnType::Fn(_) => GeneratedPyMethod::Method(impl_py_method_def(cls, &spec, None)?),
        FnType::FnClass => GeneratedPyMethod::Method(impl_py_method_def(
            cls,
            &spec,
            Some(quote!(::pyo3::ffi::METH_CLASS)),
        )?),
        FnType::FnStatic => GeneratedPyMethod::Method(impl_py_method_def(
            cls,
            &spec,
            Some(quote!(::pyo3::ffi::METH_STATIC)),
        )?),
        // special prototypes
        FnType::FnNew => GeneratedPyMethod::TraitImpl(impl_py_method_def_new(cls, &spec)?),
        FnType::FnCall(_) => GeneratedPyMethod::TraitImpl(impl_py_method_def_call(cls, &spec)?),
        FnType::ClassAttribute => GeneratedPyMethod::Method(impl_py_class_attribute(cls, &spec)),
        FnType::Getter(self_type) => GeneratedPyMethod::Method(impl_py_getter_def(
            cls,
            PropertyType::Function {
                self_type,
                spec: &spec,
            },
        )?),
        FnType::Setter(self_type) => GeneratedPyMethod::Method(impl_py_setter_def(
            cls,
            PropertyType::Function {
                self_type,
                spec: &spec,
            },
        )?),
        FnType::FnModule => {
            unreachable!("methods cannot be FnModule")
        }
    })
}

pub fn check_generic(sig: &syn::Signature) -> syn::Result<()> {
    let err_msg = |typ| format!("Python functions cannot have generic {} parameters", typ);
    for param in &sig.generics.params {
        match param {
            syn::GenericParam::Lifetime(_) => {}
            syn::GenericParam::Type(_) => bail_spanned!(param.span() => err_msg("type")),
            syn::GenericParam::Const(_) => bail_spanned!(param.span() => err_msg("const")),
        }
    }
    Ok(())
}

fn ensure_function_options_valid(options: &PyFunctionOptions) -> syn::Result<()> {
    if let Some(pass_module) = &options.pass_module {
        bail_spanned!(pass_module.span() => "`pass_module` cannot be used on Python methods");
    }
    Ok(())
}

/// Also used by pyfunction.
pub fn impl_py_method_def(
    cls: &syn::Type,
    spec: &FnSpec,
    flags: Option<TokenStream>,
) -> Result<TokenStream> {
    let wrapper_ident = syn::Ident::new("__wrap", Span::call_site());
    let wrapper_def = spec.get_wrapper_function(&wrapper_ident, Some(cls))?;
    let add_flags = flags.map(|flags| quote!(.flags(#flags)));
    let methoddef_type = match spec.tp {
        FnType::FnStatic => quote!(Static),
        FnType::FnClass => quote!(Class),
        _ => quote!(Method),
    };
    let methoddef = spec.get_methoddef(quote! {{ #wrapper_def #wrapper_ident }});
    Ok(quote! {
        ::pyo3::class::PyMethodDefType::#methoddef_type(#methoddef #add_flags)
    })
}

fn impl_py_method_def_new(cls: &syn::Type, spec: &FnSpec) -> Result<TokenStream> {
    let wrapper_ident = syn::Ident::new("__wrap", Span::call_site());
    let wrapper = spec.get_wrapper_function(&wrapper_ident, Some(cls))?;
    Ok(quote! {
        impl ::pyo3::class::impl_::PyClassNewImpl<#cls> for ::pyo3::class::impl_::PyClassImplCollector<#cls> {
            fn new_impl(self) -> ::std::option::Option<::pyo3::ffi::newfunc> {
                ::std::option::Option::Some({
                    #wrapper
                    #wrapper_ident
                })
            }
        }
    })
}

fn impl_py_method_def_call(cls: &syn::Type, spec: &FnSpec) -> Result<TokenStream> {
    let wrapper_ident = syn::Ident::new("__wrap", Span::call_site());
    let wrapper = spec.get_wrapper_function(&wrapper_ident, Some(cls))?;
    Ok(quote! {
        impl ::pyo3::class::impl_::PyClassCallImpl<#cls> for ::pyo3::class::impl_::PyClassImplCollector<#cls> {
            fn call_impl(self) -> ::std::option::Option<::pyo3::ffi::PyCFunctionWithKeywords> {
                ::std::option::Option::Some({
                    #wrapper
                    #wrapper_ident
                })
            }
        }
    })
}

fn impl_py_class_attribute(cls: &syn::Type, spec: &FnSpec) -> TokenStream {
    let name = &spec.name;
    let deprecations = &spec.deprecations;
    let python_name = spec.null_terminated_python_name();
    quote! {
        ::pyo3::class::PyMethodDefType::ClassAttribute({
            ::pyo3::class::PyClassAttributeDef::new(
                #python_name,
                ::pyo3::class::methods::PyClassAttributeFactory({
                    fn __wrap(py: ::pyo3::Python<'_>) -> ::pyo3::PyObject {
                        #deprecations
                        ::pyo3::IntoPy::into_py(#cls::#name(), py)
                    }
                    __wrap
                })
            )
        })
    }
}

fn impl_call_setter(cls: &syn::Type, spec: &FnSpec) -> syn::Result<TokenStream> {
    let (py_arg, args) = split_off_python_arg(&spec.args);

    if args.is_empty() {
        bail_spanned!(spec.name.span() => "setter function expected to have one argument");
    } else if args.len() > 1 {
        bail_spanned!(
            args[1].ty.span() =>
            "setter function can have at most two arguments ([pyo3::Python,] and value)"
        );
    }

    let name = &spec.name;
    let fncall = if py_arg.is_some() {
        quote!(#cls::#name(_slf, _py, _val))
    } else {
        quote!(#cls::#name(_slf, _val))
    };

    Ok(fncall)
}

// Used here for PropertyType::Function, used in pyclass for descriptors.
pub fn impl_py_setter_def(cls: &syn::Type, property_type: PropertyType) -> Result<TokenStream> {
    let python_name = property_type.null_terminated_python_name()?;
    let deprecations = property_type.deprecations();
    let doc = property_type.doc();
    let setter_impl = match property_type {
        PropertyType::Descriptor {
            field: syn::Field {
                ident: Some(ident), ..
            },
            ..
        } => {
            // named struct field
            quote!({ _slf.#ident = _val; })
        }
        PropertyType::Descriptor { field_index, .. } => {
            // tuple struct field
            let index = syn::Index::from(field_index);
            quote!({ _slf.#index = _val; })
        }
        PropertyType::Function { spec, .. } => impl_call_setter(cls, spec)?,
    };

    let slf = match property_type {
        PropertyType::Descriptor { .. } => {
            SelfType::Receiver { mutable: true }.receiver(cls, ExtractErrorMode::Raise)
        }
        PropertyType::Function { self_type, .. } => {
            self_type.receiver(cls, ExtractErrorMode::Raise)
        }
    };
    Ok(quote! {
        ::pyo3::class::PyMethodDefType::Setter({
            #deprecations
            ::pyo3::class::PySetterDef::new(
                #python_name,
                ::pyo3::class::methods::PySetter({
                    unsafe extern "C" fn __wrap(
                        _slf: *mut ::pyo3::ffi::PyObject,
                        _value: *mut ::pyo3::ffi::PyObject,
                        _: *mut ::std::os::raw::c_void
                    ) -> ::std::os::raw::c_int {
                        ::pyo3::callback::handle_panic(|_py| {
                            #slf
                            let _value = _py
                                .from_borrowed_ptr_or_opt(_value)
                                .ok_or_else(|| {
                                    ::pyo3::exceptions::PyAttributeError::new_err("can't delete attribute")
                                })?;
                            let _val = ::pyo3::FromPyObject::extract(_value)?;

                            ::pyo3::callback::convert(_py, #setter_impl)
                        })
                    }
                    __wrap
                }),
                #doc
            )
        })
    })
}

fn impl_call_getter(cls: &syn::Type, spec: &FnSpec) -> syn::Result<TokenStream> {
    let (py_arg, args) = split_off_python_arg(&spec.args);
    ensure_spanned!(
        args.is_empty(),
        args[0].ty.span() => "getter function can only have one argument (of type pyo3::Python)"
    );

    let name = &spec.name;
    let fncall = if py_arg.is_some() {
        quote!(#cls::#name(_slf, _py))
    } else {
        quote!(#cls::#name(_slf))
    };

    Ok(fncall)
}

// Used here for PropertyType::Function, used in pyclass for descriptors.
pub fn impl_py_getter_def(cls: &syn::Type, property_type: PropertyType) -> Result<TokenStream> {
    let python_name = property_type.null_terminated_python_name()?;
    let deprecations = property_type.deprecations();
    let doc = property_type.doc();
    let getter_impl = match property_type {
        PropertyType::Descriptor {
            field: syn::Field {
                ident: Some(ident), ..
            },
            ..
        } => {
            // named struct field
            //quote!(_slf.#ident.clone())
            quote!(::std::clone::Clone::clone(&(_slf.#ident)))
        }
        PropertyType::Descriptor { field_index, .. } => {
            // tuple struct field
            let index = syn::Index::from(field_index);
            quote!(::std::clone::Clone::clone(&(_slf.#index)))
        }
        PropertyType::Function { spec, .. } => impl_call_getter(cls, spec)?,
    };

    let slf = match property_type {
        PropertyType::Descriptor { .. } => {
            SelfType::Receiver { mutable: false }.receiver(cls, ExtractErrorMode::Raise)
        }
        PropertyType::Function { self_type, .. } => {
            self_type.receiver(cls, ExtractErrorMode::Raise)
        }
    };
    Ok(quote! {
        ::pyo3::class::PyMethodDefType::Getter({
            #deprecations
            ::pyo3::class::PyGetterDef::new(
                #python_name,
                ::pyo3::class::methods::PyGetter({
                    unsafe extern "C" fn __wrap(
                        _slf: *mut ::pyo3::ffi::PyObject,
                        _: *mut ::std::os::raw::c_void
                    ) -> *mut ::pyo3::ffi::PyObject {
                        ::pyo3::callback::handle_panic(|_py| {
                            #slf
                            ::pyo3::callback::convert(_py, #getter_impl)
                        })
                    }
                    __wrap
                }),
                #doc
            )
        })
    })
}

/// Split an argument of pyo3::Python from the front of the arg list, if present
fn split_off_python_arg<'a>(args: &'a [FnArg<'a>]) -> (Option<&FnArg>, &[FnArg]) {
    if args
        .get(0)
        .map(|py| utils::is_python(py.ty))
        .unwrap_or(false)
    {
        (Some(&args[0]), &args[1..])
    } else {
        (None, args)
    }
}

pub enum PropertyType<'a> {
    Descriptor {
        field_index: usize,
        field: &'a syn::Field,
        python_name: Option<&'a NameAttribute>,
    },
    Function {
        self_type: &'a SelfType,
        spec: &'a FnSpec<'a>,
    },
}

impl PropertyType<'_> {
    fn null_terminated_python_name(&self) -> Result<syn::LitStr> {
        match self {
            PropertyType::Descriptor {
                field, python_name, ..
            } => {
                let name = match (python_name, &field.ident) {
                    (Some(name), _) => name.0.to_string(),
                    (None, Some(field_name)) => format!("{}\0", field_name.unraw()),
                    (None, None) => {
                        bail_spanned!(field.span() => "`get` and `set` with tuple struct fields require `name`");
                    }
                };
                Ok(syn::LitStr::new(&name, field.span()))
            }
            PropertyType::Function { spec, .. } => Ok(spec.null_terminated_python_name()),
        }
    }

    fn deprecations(&self) -> Option<&Deprecations> {
        match self {
            PropertyType::Descriptor { .. } => None,
            PropertyType::Function { spec, .. } => Some(&spec.deprecations),
        }
    }

    fn doc(&self) -> Cow<PythonDoc> {
        match self {
            PropertyType::Descriptor { field, .. } => {
                Cow::Owned(utils::get_doc(&field.attrs, None))
            }
            PropertyType::Function { spec, .. } => Cow::Borrowed(&spec.doc),
        }
    }
}

const __GETATTR__: SlotDef = SlotDef::new("Py_tp_getattro", "getattrofunc")
    .arguments(&[Ty::Object])
    .before_call_method(TokenGenerator(|| {
        quote! {
            // Behave like python's __getattr__ (as opposed to __getattribute__) and check
            // for existing fields and methods first
            let existing = ::pyo3::ffi::PyObject_GenericGetAttr(_slf, arg0);
            if existing.is_null() {
                // PyObject_HasAttr also tries to get an object and clears the error if it fails
                ::pyo3::ffi::PyErr_Clear();
            } else {
                return existing;
            }
        }
    }));
const __STR__: SlotDef = SlotDef::new("Py_tp_str", "reprfunc");
const __REPR__: SlotDef = SlotDef::new("Py_tp_repr", "reprfunc");
const __HASH__: SlotDef = SlotDef::new("Py_tp_hash", "hashfunc")
    .ret_ty(Ty::PyHashT)
    .return_conversion(TokenGenerator(
        || quote! { ::pyo3::callback::HashCallbackOutput },
    ));
const __RICHCMP__: SlotDef = SlotDef::new("Py_tp_richcompare", "richcmpfunc")
    .extract_error_mode(ExtractErrorMode::NotImplemented)
    .arguments(&[Ty::ObjectOrNotImplemented, Ty::CompareOp]);
const __GET__: SlotDef =
    SlotDef::new("Py_tp_descr_get", "descrgetfunc").arguments(&[Ty::Object, Ty::Object]);
const __ITER__: SlotDef = SlotDef::new("Py_tp_iter", "getiterfunc");
const __NEXT__: SlotDef = SlotDef::new("Py_tp_iternext", "iternextfunc").return_conversion(
    TokenGenerator(|| quote! { ::pyo3::class::iter::IterNextOutput::<_, _> }),
);
const __AWAIT__: SlotDef = SlotDef::new("Py_am_await", "unaryfunc");
const __AITER__: SlotDef = SlotDef::new("Py_am_aiter", "unaryfunc");
const __ANEXT__: SlotDef = SlotDef::new("Py_am_anext", "unaryfunc").return_conversion(
    TokenGenerator(|| quote! { ::pyo3::class::pyasync::IterANextOutput::<_, _> }),
);
const __LEN__: SlotDef = SlotDef::new("Py_mp_length", "lenfunc").ret_ty(Ty::PySsizeT);
const __CONTAINS__: SlotDef = SlotDef::new("Py_sq_contains", "objobjproc")
    .arguments(&[Ty::Object])
    .ret_ty(Ty::Int);
const __GETITEM__: SlotDef = SlotDef::new("Py_mp_subscript", "binaryfunc").arguments(&[Ty::Object]);

const __POS__: SlotDef = SlotDef::new("Py_nb_positive", "unaryfunc");
const __NEG__: SlotDef = SlotDef::new("Py_nb_negative", "unaryfunc");
const __ABS__: SlotDef = SlotDef::new("Py_nb_absolute", "unaryfunc");
const __INVERT__: SlotDef = SlotDef::new("Py_nb_invert", "unaryfunc");
const __INDEX__: SlotDef = SlotDef::new("Py_nb_index", "unaryfunc");
const __INT__: SlotDef = SlotDef::new("Py_nb_int", "unaryfunc");
const __FLOAT__: SlotDef = SlotDef::new("Py_nb_float", "unaryfunc");
const __BOOL__: SlotDef = SlotDef::new("Py_nb_bool", "inquiry").ret_ty(Ty::Int);

const __TRUEDIV__: SlotDef = SlotDef::new("Py_nb_true_divide", "binaryfunc")
    .arguments(&[Ty::ObjectOrNotImplemented])
    .extract_error_mode(ExtractErrorMode::NotImplemented);
const __FLOORDIV__: SlotDef = SlotDef::new("Py_nb_floor_divide", "binaryfunc")
    .arguments(&[Ty::ObjectOrNotImplemented])
    .extract_error_mode(ExtractErrorMode::NotImplemented);

const __IADD__: SlotDef = SlotDef::new("Py_nb_inplace_add", "binaryfunc")
    .arguments(&[Ty::ObjectOrNotImplemented])
    .extract_error_mode(ExtractErrorMode::NotImplemented)
    .return_self();
const __ISUB__: SlotDef = SlotDef::new("Py_nb_inplace_subtract", "binaryfunc")
    .arguments(&[Ty::ObjectOrNotImplemented])
    .extract_error_mode(ExtractErrorMode::NotImplemented)
    .return_self();
const __IMUL__: SlotDef = SlotDef::new("Py_nb_inplace_multiply", "binaryfunc")
    .arguments(&[Ty::ObjectOrNotImplemented])
    .extract_error_mode(ExtractErrorMode::NotImplemented)
    .return_self();
const __IMATMUL__: SlotDef = SlotDef::new("Py_nb_inplace_matrix_multiply", "binaryfunc")
    .arguments(&[Ty::ObjectOrNotImplemented])
    .extract_error_mode(ExtractErrorMode::NotImplemented)
    .return_self();
const __ITRUEDIV__: SlotDef = SlotDef::new("Py_nb_inplace_true_divide", "binaryfunc")
    .arguments(&[Ty::ObjectOrNotImplemented])
    .extract_error_mode(ExtractErrorMode::NotImplemented)
    .return_self();
const __IFLOORDIV__: SlotDef = SlotDef::new("Py_nb_inplace_floor_divide", "binaryfunc")
    .arguments(&[Ty::ObjectOrNotImplemented])
    .extract_error_mode(ExtractErrorMode::NotImplemented)
    .return_self();
const __IMOD__: SlotDef = SlotDef::new("Py_nb_inplace_remainder", "binaryfunc")
    .arguments(&[Ty::ObjectOrNotImplemented])
    .extract_error_mode(ExtractErrorMode::NotImplemented)
    .return_self();
const __IPOW__: SlotDef = SlotDef::new("Py_nb_inplace_power", "ternaryfunc")
    .arguments(&[Ty::ObjectOrNotImplemented, Ty::ObjectOrNotImplemented])
    .extract_error_mode(ExtractErrorMode::NotImplemented)
    .return_self();
const __ILSHIFT__: SlotDef = SlotDef::new("Py_nb_inplace_lshift", "binaryfunc")
    .arguments(&[Ty::ObjectOrNotImplemented])
    .extract_error_mode(ExtractErrorMode::NotImplemented)
    .return_self();
const __IRSHIFT__: SlotDef = SlotDef::new("Py_nb_inplace_rshift", "binaryfunc")
    .arguments(&[Ty::ObjectOrNotImplemented])
    .extract_error_mode(ExtractErrorMode::NotImplemented)
    .return_self();
const __IAND__: SlotDef = SlotDef::new("Py_nb_inplace_and", "binaryfunc")
    .arguments(&[Ty::ObjectOrNotImplemented])
    .extract_error_mode(ExtractErrorMode::NotImplemented)
    .return_self();
const __IXOR__: SlotDef = SlotDef::new("Py_nb_inplace_xor", "binaryfunc")
    .arguments(&[Ty::ObjectOrNotImplemented])
    .extract_error_mode(ExtractErrorMode::NotImplemented)
    .return_self();
const __IOR__: SlotDef = SlotDef::new("Py_nb_inplace_or", "binaryfunc")
    .arguments(&[Ty::ObjectOrNotImplemented])
    .extract_error_mode(ExtractErrorMode::NotImplemented)
    .return_self();

fn pyproto(method_name: &str) -> Option<&'static SlotDef> {
    match method_name {
        "__getattr__" => Some(&__GETATTR__),
        "__str__" => Some(&__STR__),
        "__repr__" => Some(&__REPR__),
        "__hash__" => Some(&__HASH__),
        "__richcmp__" => Some(&__RICHCMP__),
        "__get__" => Some(&__GET__),
        "__iter__" => Some(&__ITER__),
        "__next__" => Some(&__NEXT__),
        "__await__" => Some(&__AWAIT__),
        "__aiter__" => Some(&__AITER__),
        "__anext__" => Some(&__ANEXT__),
        "__len__" => Some(&__LEN__),
        "__contains__" => Some(&__CONTAINS__),
        "__getitem__" => Some(&__GETITEM__),
        "__pos__" => Some(&__POS__),
        "__neg__" => Some(&__NEG__),
        "__abs__" => Some(&__ABS__),
        "__invert__" => Some(&__INVERT__),
        "__index__" => Some(&__INDEX__),
        "__int__" => Some(&__INT__),
        "__float__" => Some(&__FLOAT__),
        "__bool__" => Some(&__BOOL__),
        "__truediv__" => Some(&__TRUEDIV__),
        "__floordiv__" => Some(&__FLOORDIV__),
        "__iadd__" => Some(&__IADD__),
        "__isub__" => Some(&__ISUB__),
        "__imul__" => Some(&__IMUL__),
        "__imatmul__" => Some(&__IMATMUL__),
        "__itruediv__" => Some(&__ITRUEDIV__),
        "__ifloordiv__" => Some(&__IFLOORDIV__),
        "__imod__" => Some(&__IMOD__),
        "__ipow__" => Some(&__IPOW__),
        "__ilshift__" => Some(&__ILSHIFT__),
        "__irshift__" => Some(&__IRSHIFT__),
        "__iand__" => Some(&__IAND__),
        "__ixor__" => Some(&__IXOR__),
        "__ior__" => Some(&__IOR__),
        _ => None,
    }
}

#[derive(Clone, Copy)]
enum Ty {
    Object,
    ObjectOrNotImplemented,
    NonNullObject,
    CompareOp,
    Int,
    PyHashT,
    PySsizeT,
    Void,
}

impl Ty {
    fn ffi_type(self) -> TokenStream {
        match self {
            Ty::Object | Ty::ObjectOrNotImplemented => quote! { *mut ::pyo3::ffi::PyObject },
            Ty::NonNullObject => quote! { ::std::ptr::NonNull<::pyo3::ffi::PyObject> },
            Ty::Int | Ty::CompareOp => quote! { ::std::os::raw::c_int },
            Ty::PyHashT => quote! { ::pyo3::ffi::Py_hash_t },
            Ty::PySsizeT => quote! { ::pyo3::ffi::Py_ssize_t },
            Ty::Void => quote! { () },
        }
    }

    fn extract(
        self,
        cls: &syn::Type,
        py: &syn::Ident,
        ident: &syn::Ident,
        target: &syn::Type,
    ) -> TokenStream {
        match self {
            Ty::Object => {
                let extract = extract_from_any(cls, target, ident);
                quote! {
                    let #ident: &::pyo3::PyAny = #py.from_borrowed_ptr(#ident);
                    #extract
                }
            }
            Ty::ObjectOrNotImplemented => {
                let extract = if let syn::Type::Reference(tref) = unwrap_ty_group(target) {
                    let (tref, mut_) = preprocess_tref(tref, cls);
                    quote! {
                        let #mut_ #ident: <#tref as ::pyo3::derive_utils::ExtractExt<'_>>::Target = match #ident.extract() {
                            Ok(#ident) => #ident,
                            Err(_) => return ::pyo3::callback::convert(#py, #py.NotImplemented()),
                        };
                        let #ident = &#mut_ *#ident;
                    }
                } else {
                    quote! {
                        let #ident = match #ident.extract() {
                            Ok(#ident) => #ident,
                            Err(_) => return ::pyo3::callback::convert(#py, #py.NotImplemented()),
                        };
                    }
                };
                quote! {
                    let #ident: &::pyo3::PyAny = #py.from_borrowed_ptr(#ident);
                    #extract
                }
            }
            Ty::NonNullObject => {
                let extract = extract_from_any(cls, target, ident);
                quote! {
                    let #ident: &::pyo3::PyAny = #py.from_borrowed_ptr(#ident.as_ptr());
                    #extract
                }
            }
            Ty::CompareOp => quote! {
                let #ident = ::pyo3::class::basic::CompareOp::from_raw(#ident)
                    .ok_or_else(|| ::pyo3::exceptions::PyValueError::new_err("invalid comparison operator"))?;
            },
            Ty::Int | Ty::PyHashT | Ty::PySsizeT | Ty::Void => todo!(),
        }
    }
}

fn extract_from_any(self_: &syn::Type, target: &syn::Type, ident: &syn::Ident) -> TokenStream {
    return if let syn::Type::Reference(tref) = unwrap_ty_group(target) {
        let (tref, mut_) = preprocess_tref(tref, self_);
        quote! {
            let #mut_ #ident: <#tref as ::pyo3::derive_utils::ExtractExt<'_>>::Target = #ident.extract()?;
            let #ident = &#mut_ *#ident;
        }
    } else {
        quote! {
            let #ident = #ident.extract()?;
        }
    };
}

/// Replace `Self`, remove lifetime and get mutability from the type
fn preprocess_tref(
    tref: &syn::TypeReference,
    self_: &syn::Type,
) -> (syn::TypeReference, Option<syn::token::Mut>) {
    let mut tref = tref.to_owned();
    if let syn::Type::Path(tpath) = self_ {
        replace_self(&mut tref, &tpath.path);
    }
    tref.lifetime = None;
    let mut_ = tref.mutability;
    (tref, mut_)
}

/// Replace `Self` with the exact type name since it is used out of the impl block
fn replace_self(tref: &mut syn::TypeReference, self_path: &syn::Path) {
    match &mut *tref.elem {
        syn::Type::Reference(tref_inner) => replace_self(tref_inner, self_path),
        syn::Type::Path(tpath) => {
            if let Some(ident) = tpath.path.get_ident() {
                if ident == "Self" {
                    tpath.path = self_path.to_owned();
                }
            }
        }
        _ => {}
    }
}

enum ReturnMode {
    ReturnSelf,
    Conversion(TokenGenerator),
}

impl ReturnMode {
    fn return_call_output(&self, py: &syn::Ident, call: TokenStream) -> TokenStream {
        match self {
            ReturnMode::Conversion(conversion) => quote! {
                let _result: PyResult<#conversion> = #call;
                ::pyo3::callback::convert(#py, _result)
            },
            ReturnMode::ReturnSelf => quote! {
                let _result: PyResult<()> = #call;
                _result?;
                ::pyo3::ffi::Py_XINCREF(_raw_slf);
                Ok(_raw_slf)
            },
        }
    }
}

struct SlotDef {
    slot: StaticIdent,
    func_ty: StaticIdent,
    arguments: &'static [Ty],
    ret_ty: Ty,
    before_call_method: Option<TokenGenerator>,
    extract_error_mode: ExtractErrorMode,
    return_mode: Option<ReturnMode>,
}

const NO_ARGUMENTS: &[Ty] = &[];

impl SlotDef {
    const fn new(slot: &'static str, func_ty: &'static str) -> Self {
        SlotDef {
            slot: StaticIdent(slot),
            func_ty: StaticIdent(func_ty),
            arguments: NO_ARGUMENTS,
            ret_ty: Ty::Object,
            before_call_method: None,
            extract_error_mode: ExtractErrorMode::Raise,
            return_mode: None,
        }
    }

    const fn arguments(mut self, arguments: &'static [Ty]) -> Self {
        self.arguments = arguments;
        self
    }

    const fn ret_ty(mut self, ret_ty: Ty) -> Self {
        self.ret_ty = ret_ty;
        self
    }

    const fn before_call_method(mut self, before_call_method: TokenGenerator) -> Self {
        self.before_call_method = Some(before_call_method);
        self
    }

    const fn return_conversion(mut self, return_conversion: TokenGenerator) -> Self {
        self.return_mode = Some(ReturnMode::Conversion(return_conversion));
        self
    }

    const fn extract_error_mode(mut self, extract_error_mode: ExtractErrorMode) -> Self {
        self.extract_error_mode = extract_error_mode;
        self
    }

    const fn return_self(mut self) -> Self {
        self.return_mode = Some(ReturnMode::ReturnSelf);
        self
    }

    fn generate_type_slot(&self, cls: &syn::Type, spec: &FnSpec) -> Result<TokenStream> {
        let SlotDef {
            slot,
            func_ty,
            before_call_method,
            arguments,
            extract_error_mode,
            ret_ty,
            return_mode,
        } = self;
        let py = syn::Ident::new("_py", Span::call_site());
        let method_arguments = generate_method_arguments(arguments);
        let ret_ty = ret_ty.ffi_type();
        let body = generate_method_body(
            cls,
            spec,
            &py,
            arguments,
            *extract_error_mode,
            return_mode.as_ref(),
        )?;
        Ok(quote!({
            unsafe extern "C" fn __wrap(_raw_slf: *mut ::pyo3::ffi::PyObject, #(#method_arguments),*) -> #ret_ty {
                let _slf = _raw_slf;
                #before_call_method
                ::pyo3::callback::handle_panic(|#py| {
                    #body
                })
            }
            ::pyo3::ffi::PyType_Slot {
                slot: ::pyo3::ffi::#slot,
                pfunc: __wrap as ::pyo3::ffi::#func_ty as _
            }
        }))
    }
}

fn generate_method_arguments(arguments: &[Ty]) -> impl Iterator<Item = TokenStream> + '_ {
    arguments.iter().enumerate().map(|(i, arg)| {
        let ident = syn::Ident::new(&format!("arg{}", i), Span::call_site());
        let ffi_type = arg.ffi_type();
        quote! {
            #ident: #ffi_type
        }
    })
}

fn generate_method_body(
    cls: &syn::Type,
    spec: &FnSpec,
    py: &syn::Ident,
    arguments: &[Ty],
    extract_error_mode: ExtractErrorMode,
    return_mode: Option<&ReturnMode>,
) -> Result<TokenStream> {
    let self_conversion = spec.tp.self_conversion(Some(cls), extract_error_mode);
    let rust_name = spec.name;
    let (arg_idents, conversions) = extract_proto_arguments(cls, py, &spec.args, arguments)?;
    let call = quote! { ::pyo3::callback::convert(#py, #cls::#rust_name(_slf, #(#arg_idents),*)) };
    let body = if let Some(return_mode) = return_mode {
        return_mode.return_call_output(py, call)
    } else {
        call
    };
    Ok(quote! {
        #self_conversion
        #conversions
        #body
    })
}

struct SlotFragmentDef {
    fragment: &'static str,
    arguments: &'static [Ty],
    extract_error_mode: ExtractErrorMode,
    ret_ty: Ty,
}

impl SlotFragmentDef {
    const fn new(fragment: &'static str, arguments: &'static [Ty]) -> Self {
        SlotFragmentDef {
            fragment,
            arguments,
            extract_error_mode: ExtractErrorMode::Raise,
            ret_ty: Ty::Void,
        }
    }

    const fn extract_error_mode(mut self, extract_error_mode: ExtractErrorMode) -> Self {
        self.extract_error_mode = extract_error_mode;
        self
    }

    const fn ret_ty(mut self, ret_ty: Ty) -> Self {
        self.ret_ty = ret_ty;
        self
    }

    fn generate_pyproto_fragment(&self, cls: &syn::Type, spec: &FnSpec) -> Result<TokenStream> {
        let SlotFragmentDef {
            fragment,
            arguments,
            extract_error_mode,
            ret_ty,
        } = self;
        let fragment_trait = format_ident!("PyClass{}SlotFragment", fragment);
        let method = syn::Ident::new(fragment, Span::call_site());
        let py = syn::Ident::new("_py", Span::call_site());
        let method_arguments = generate_method_arguments(arguments);
        let body = generate_method_body(cls, spec, &py, arguments, *extract_error_mode, None)?;
        let ret_ty = ret_ty.ffi_type();
        Ok(quote! {
            impl ::pyo3::class::impl_::#fragment_trait<#cls> for ::pyo3::class::impl_::PyClassImplCollector<#cls> {

                #[inline]
                unsafe fn #method(
                    self,
                    #py: ::pyo3::Python,
                    _raw_slf: *mut ::pyo3::ffi::PyObject,
                    #(#method_arguments),*
                ) -> ::pyo3::PyResult<#ret_ty> {
                    let _slf = _raw_slf;
                    #body
                }
            }
        })
    }
}

const __SETATTR__: SlotFragmentDef =
    SlotFragmentDef::new("__setattr__", &[Ty::Object, Ty::NonNullObject]);
const __DELATTR__: SlotFragmentDef = SlotFragmentDef::new("__delattr__", &[Ty::Object]);
const __SET__: SlotFragmentDef = SlotFragmentDef::new("__set__", &[Ty::Object, Ty::NonNullObject]);
const __DELETE__: SlotFragmentDef = SlotFragmentDef::new("__delete__", &[Ty::Object]);
const __SETITEM__: SlotFragmentDef =
    SlotFragmentDef::new("__setitem__", &[Ty::Object, Ty::NonNullObject]);
const __DELITEM__: SlotFragmentDef = SlotFragmentDef::new("__delitem__", &[Ty::Object]);

macro_rules! binary_num_slot_fragment_def {
    ($ident:ident, $name:literal) => {
        const $ident: SlotFragmentDef = SlotFragmentDef::new($name, &[Ty::ObjectOrNotImplemented])
            .extract_error_mode(ExtractErrorMode::NotImplemented)
            .ret_ty(Ty::Object);
    };
}

binary_num_slot_fragment_def!(__ADD__, "__add__");
binary_num_slot_fragment_def!(__RADD__, "__radd__");
binary_num_slot_fragment_def!(__SUB__, "__sub__");
binary_num_slot_fragment_def!(__RSUB__, "__rsub__");
binary_num_slot_fragment_def!(__MUL__, "__mul__");
binary_num_slot_fragment_def!(__RMUL__, "__rmul__");
binary_num_slot_fragment_def!(__MATMUL__, "__matmul__");
binary_num_slot_fragment_def!(__RMATMUL__, "__rmatmul__");
binary_num_slot_fragment_def!(__DIVMOD__, "__divmod__");
binary_num_slot_fragment_def!(__RDIVMOD__, "__rdivmod__");
binary_num_slot_fragment_def!(__MOD__, "__mod__");
binary_num_slot_fragment_def!(__RMOD__, "__rmod__");
binary_num_slot_fragment_def!(__LSHIFT__, "__lshift__");
binary_num_slot_fragment_def!(__RLSHIFT__, "__rlshift__");
binary_num_slot_fragment_def!(__RSHIFT__, "__rshift__");
binary_num_slot_fragment_def!(__RRSHIFT__, "__rrshift__");
binary_num_slot_fragment_def!(__AND__, "__and__");
binary_num_slot_fragment_def!(__RAND__, "__rand__");
binary_num_slot_fragment_def!(__XOR__, "__xor__");
binary_num_slot_fragment_def!(__RXOR__, "__rxor__");
binary_num_slot_fragment_def!(__OR__, "__or__");
binary_num_slot_fragment_def!(__ROR__, "__ror__");

const __POW__: SlotFragmentDef = SlotFragmentDef::new(
    "__pow__",
    &[Ty::ObjectOrNotImplemented, Ty::ObjectOrNotImplemented],
)
.extract_error_mode(ExtractErrorMode::NotImplemented)
.ret_ty(Ty::Object);
const __RPOW__: SlotFragmentDef = SlotFragmentDef::new(
    "__rpow__",
    &[Ty::ObjectOrNotImplemented, Ty::ObjectOrNotImplemented],
)
.extract_error_mode(ExtractErrorMode::NotImplemented)
.ret_ty(Ty::Object);

fn pyproto_fragment(method_name: &str) -> Option<&'static SlotFragmentDef> {
    match method_name {
        "__setattr__" => Some(&__SETATTR__),
        "__delattr__" => Some(&__DELATTR__),
        "__set__" => Some(&__SET__),
        "__delete__" => Some(&__DELETE__),
        "__setitem__" => Some(&__SETITEM__),
        "__delitem__" => Some(&__DELITEM__),
        "__add__" => Some(&__ADD__),
        "__radd__" => Some(&__RADD__),
        "__sub__" => Some(&__SUB__),
        "__rsub__" => Some(&__RSUB__),
        "__mul__" => Some(&__MUL__),
        "__rmul__" => Some(&__RMUL__),
        "__matmul__" => Some(&__MATMUL__),
        "__rmatmul__" => Some(&__RMATMUL__),
        "__divmod__" => Some(&__DIVMOD__),
        "__rdivmod__" => Some(&__RDIVMOD__),
        "__mod__" => Some(&__MOD__),
        "__rmod__" => Some(&__RMOD__),
        "__lshift__" => Some(&__LSHIFT__),
        "__rlshift__" => Some(&__RLSHIFT__),
        "__rshift__" => Some(&__RSHIFT__),
        "__rrshift__" => Some(&__RRSHIFT__),
        "__and__" => Some(&__AND__),
        "__rand__" => Some(&__RAND__),
        "__xor__" => Some(&__XOR__),
        "__rxor__" => Some(&__RXOR__),
        "__or__" => Some(&__OR__),
        "__ror__" => Some(&__ROR__),
        "__pow__" => Some(&__POW__),
        "__rpow__" => Some(&__RPOW__),
        _ => None,
    }
}

fn extract_proto_arguments(
    cls: &syn::Type,
    py: &syn::Ident,
    method_args: &[FnArg],
    proto_args: &[Ty],
) -> Result<(Vec<Ident>, TokenStream)> {
    let mut arg_idents = Vec::with_capacity(method_args.len());
    let mut non_python_args = 0;

    let mut args_conversions = Vec::with_capacity(proto_args.len());

    for arg in method_args {
        if arg.py {
            arg_idents.push(py.clone());
        } else {
            let ident = syn::Ident::new(&format!("arg{}", non_python_args), Span::call_site());
            let conversions = proto_args.get(non_python_args)
                .ok_or_else(|| err_spanned!(arg.ty.span() => format!("Expected at most {} non-python arguments", proto_args.len())))?
                .extract(cls, py, &ident, arg.ty);
            non_python_args += 1;
            args_conversions.push(conversions);
            arg_idents.push(ident);
        }
    }

    let conversions = quote!(#(#args_conversions)*);
    Ok((arg_idents, conversions))
}

struct StaticIdent(&'static str);

impl ToTokens for StaticIdent {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        syn::Ident::new(self.0, Span::call_site()).to_tokens(tokens)
    }
}

struct TokenGenerator(fn() -> TokenStream);

impl ToTokens for TokenGenerator {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        self.0().to_tokens(tokens)
    }
}

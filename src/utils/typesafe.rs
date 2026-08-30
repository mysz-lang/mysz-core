use std::collections::HashMap;

use crate::{parse::parsing::Type, semantics::analysis::StructSignature};

pub fn mangle_name(base_name: &str, args: &[Type]) -> String {
    let mut name = base_name.to_string();
    for arg in args {
        name.push_str("__");
        name.push_str(&type_to_mangled_string(arg));
    }
    name
}

pub fn type_to_mangled_string(ty: &Type) -> String {
    match ty {
        Type::Int => "int".to_string(),
        Type::UInt => "uint".to_string(),
        Type::Int8 => "int8".to_string(),
        Type::UInt8 => "uint8".to_string(),
        Type::Float => "float".to_string(),
        Type::Double => "double".to_string(),
        Type::Bool => "bool".to_string(),
        Type::Str => "str".to_string(),
        Type::Char => "char".to_string(),
        Type::Void => "void".to_string(),
        Type::Any => "any".to_string(),
        Type::Nil => "nil".to_string(),
        Type::Struct(name) => name.clone(),
        Type::Enum(name) => name.clone(),
        Type::Ptr(inner) => format!("ptr__{}", type_to_mangled_string(inner)),
        Type::Array { element_type, size } => {
            format!("arr__{}__{}", type_to_mangled_string(element_type), size)
        }
        Type::GenericInstance { name, args } => {
            let mut base = name.clone();
            for arg in args {
                base.push_str("__");
                base.push_str(&type_to_mangled_string(arg));
            }
            base
        }
        Type::GenericParam(s) => format!("gparam__{}", s),
        Type::VariadicPack { .. } => {
            unreachable!(
                "VariadicPack is a symbolic placeholder and should never be mangled directly \
                 — a concrete arg type was expected here. This indicates a compiler bug."
            )
        }
    }
}

pub fn mangle_variadic(base_mangled_name: &str, variadic_types: &[Type]) -> String {
    if variadic_types.is_empty() {
        format!("{}.", base_mangled_name)
    } else {
        let joined = variadic_types
            .iter()
            .map(type_to_mangled_string)
            .collect::<Vec<_>>()
            .join("__");
        format!("{}.{}", base_mangled_name, joined)
    }
}

pub fn normalise_type(ty: &Type) -> Type {
    match ty {
        Type::GenericInstance { name, args } => {
            let mangled_name = mangle_name(name, args);
            Type::Struct(mangled_name)
        }
        Type::Ptr(inner) => Type::Ptr(Box::new(normalise_type(inner))),
        Type::Array { element_type, size } => Type::Array {
            element_type: Box::new(normalise_type(element_type)),
            size: *size,
        },
        _ => ty.clone(),
    }
}

pub fn both_way_allow(found: &Type, expected: &Type, a: Type, b: Type) -> bool {
    (found == &a && expected == &b)
        || (found == &b && expected == &a)
        || (found == &a && expected == &a)
        || (found == &b && expected == &b)
}

#[inline]
pub fn is_integer(ty: &Type) -> bool {
    matches!(
        ty,
        Type::Int | Type::UInt | Type::Int8 | Type::UInt8 | Type::Char
    )
}

#[inline]
pub fn is_signed_integer(ty: &Type) -> bool {
    matches!(ty, Type::Int | Type::Int8)
}

#[inline]
pub fn is_unsigned_integer(ty: &Type) -> bool {
    matches!(ty, Type::UInt | Type::UInt8 | Type::Char)
}

#[inline]
pub fn is_decimal(ty: &Type) -> bool {
    matches!(ty, Type::Float | Type::Double)
}

#[inline]
pub fn is_truthy_type(ty: &Type) -> bool {
    matches!(
        ty,
        Type::Int
            | Type::UInt
            | Type::Int8
            | Type::UInt8
            | Type::Bool
            | Type::Str
            | Type::Enum(..)
            | Type::Nil
    )
}

#[derive(Clone, Copy)]
pub enum TypeCheckMode {
    Strict,   // exact match only
    Coercive, // allows implicit conversions
    Passive,  // allows any conversion
}

pub fn types_match(expected: &Type, found: &Type, mode: TypeCheckMode) -> bool {
    if expected == &Type::Any || found == &Type::Any {
        return true;
    }

    if expected == &Type::Nil || found == &Type::Nil {
        return true;
    }

    if both_way_allow(found, expected, Type::Int, Type::Enum("".to_string())) {
        return true;
    };

    if matches!(expected, _found) {
        return true;
    }
    match mode {
        TypeCheckMode::Strict => false,
        TypeCheckMode::Coercive => {
            if is_integer(expected) && is_integer(found) {
                return true;
            }

            if both_way_allow(found, expected, Type::Ptr(Box::new(Type::Char)), Type::Str) {
                return true;
            };

            if both_way_allow(found, expected, Type::Char, Type::UInt8) {
                return true;
            };

            if is_decimal(expected) && is_decimal(found) {
                return true;
            }

            if is_integer(found) && is_decimal(expected) {
                return true;
            }
            if is_integer(expected) && is_decimal(found) {
                return true;
            }

            false
        }
        TypeCheckMode::Passive => true,
    }
}

pub fn types_compatible(expected: &Type, from: &Type) -> bool {
    types_match(expected, from, TypeCheckMode::Coercive)
}

pub fn types_equal(expected: &Type, from: &Type) -> bool {
    types_match(expected, from, TypeCheckMode::Strict)
}

pub fn type_to_string(ty: &Type) -> String {
    match ty {
        Type::Struct(name) => name.clone(),
        Type::Ptr(inner) => format!("ptr<{}>", type_to_string(inner)),
        Type::Array { element_type, size } => {
            format!("[{}; {}]", type_to_string(element_type), size)
        }
        Type::GenericInstance { name, args } => {
            let args_str: Vec<String> = args.iter().map(type_to_string).collect();
            format!("{}<{}>", name, args_str.join(", "))
        }
        Type::GenericParam(s) => format!("<{}>", s),

        _ => typeof_string(ty),
    }
}

pub fn typeof_string(ty: &Type) -> String {
    match ty {
        Type::Int => "int".to_string(),
        Type::UInt => "uint".to_string(),
        Type::Int8 => "i8".to_string(),
        Type::UInt8 => "u8".to_string(),
        Type::Float => "float".to_string(),
        Type::Double => "double".to_string(),
        Type::Bool => "bool".to_string(),
        Type::Str => "str".to_string(),
        Type::Char => "char".to_string(),
        Type::Void => "void".to_string(),
        Type::Any => "any".to_string(),
        Type::Nil => "nil".to_string(),
        Type::Struct(s) => s.to_string(),
        Type::Enum(s) => s.to_string(),
        Type::Ptr(s) => format!("ptr<{}>", typeof_string(s.as_ref())),
        Type::Array { element_type, .. } => format!("[{}]", typeof_string(element_type.as_ref())),
        Type::GenericInstance { .. } => "generic_instance".to_string(),
        Type::GenericParam(..) => "generic_param".to_string(),
        Type::VariadicPack { .. } => "variadic".to_string(),
    }
}

pub mod variadic {
    use indexmap::IndexMap;

    use crate::semantics::analysis::StructSignature;
    use crate::utils::location::Location;
    use crate::utils::typesafe::Type;

    pub fn structure(fields: &[Type], location: Location) -> StructSignature {
        let mut struct_fields = IndexMap::new();

        for (i, ty) in fields.iter().enumerate() {
            struct_fields.insert(field_name(i), ty.clone());
        }

        struct_fields.insert(length_field().to_string(), Type::Int);

        StructSignature {
            generic_params: Vec::new(),
            fields: struct_fields,
            location,
        }
    }

    pub fn field_name(index: usize) -> String {
        format!("i{}", index)
    }

    pub fn length_field() -> &'static str {
        "il"
    }

    pub enum PackField {
        /// `pack.iN`: the Nth variadic argument.
        Index(usize),
        /// `pack.il`: the number of variadic arguments passed.
        Length,
    }

    /// Parses a field name against the pack naming convention, without
    /// needing to know the concrete argument types. This is what makes it
    /// possible to type-check `a.i0` inside the *generic*, un-instantiated
    /// body of a variadic function/proc, where the real element types and
    /// count aren't known yet (they're only known at each call site).
    pub fn parse_field(field: &str) -> Option<PackField> {
        if field == length_field() {
            return Some(PackField::Length);
        }
        field
            .strip_prefix('i')
            .and_then(|rest| rest.parse::<usize>().ok())
            .map(PackField::Index)
    }
}

pub fn is_iterable(ty: &Type) -> bool {
    matches!(
        ty,
        Type::Struct(_) | Type::GenericInstance { .. } | Type::VariadicPack { .. }
    )
}

pub fn iterable_elements(
    ty: &Type,
    structs: &HashMap<String, StructSignature>,
) -> Option<Vec<(Option<String>, Type)>> {
    match ty {
        Type::Struct(name) => {
            let sig = structs.get(name)?;
            let mut result = Vec::new();
            for (fname, ftype) in sig.fields.iter() {
                result.push((Some(fname.clone()), ftype.clone()));
            }
            Some(result)
        }
        Type::GenericInstance { name, args } => {
            let concrete_name = crate::utils::typesafe::mangle_name(name, args);
            let sig = structs.get(&concrete_name)?;
            let mut result = Vec::new();
            for (fname, ftype) in sig.fields.iter() {
                result.push((Some(fname.clone()), ftype.clone()));
            }
            Some(result)
        }
        Type::VariadicPack { types, .. } => {
            let mut result = Vec::new();
            for (i, ty) in types.iter().enumerate() {
                result.push((Some(format!("i{}", i)), ty.clone()));
            }
            Some(result)
        }
        _ => None,
    }
}

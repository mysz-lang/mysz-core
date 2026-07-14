// --- Modern Type Classification & Introspection Helpers ---

use crate::parse::parsing::Type;

pub fn mangle_name(base_name: &str, args: &[Type]) -> String {
    let mut name = base_name.to_string();
    for arg in args {
        name.push_str("__");
        // Use flat representation for compiler-internal mangling to keep symbols ASCII-safe
        name.push_str(&type_to_mangled_string(arg));
    }
    name
}

pub fn type_to_mangled_string(ty: &Type) -> String {
    match ty {
        Type::Int => "int".to_string(),
        Type::UInt => "uint".to_string(),
        Type::Bool => "bool".to_string(),
        Type::Str => "str".to_string(),
        Type::Char => "char".to_string(),
        Type::Void => "void".to_string(),
        Type::Any => "any".to_string(),
        Type::Struct(name) => name.clone(),
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

#[inline]
pub fn is_integer(ty: &Type) -> bool {
    matches!(ty, Type::Int | Type::UInt)
}

#[inline]
pub fn is_signed_integer(ty: &Type) -> bool {
    matches!(ty, Type::Int)
}

#[inline]
pub fn is_truthy_type(ty: &Type) -> bool {
    matches!(ty, Type::Int | Type::UInt | Type::Bool | Type::Str)
}

/// Determines if a source type (`from`) can be safely coerced or matched to an `expected` destination type.
pub fn types_compatible(expected: &Type, from: &Type) -> bool {
    if expected == &Type::Any || from == &Type::Any {
        return true;
    }

    let norm_expected = normalise_type(expected);
    let norm_from = normalise_type(from);

    if norm_expected == norm_from {
        return true;
    }

    if is_integer(&norm_from) && is_integer(&norm_expected) {
        return true;
    }

    false
}

/// Recursively formats a compiler Type to a clean, user-friendly language representation.
pub fn type_to_string(ty: &Type) -> String {
    match ty {
        Type::Int => "int".to_string(),
        Type::UInt => "uint".to_string(),
        Type::Bool => "bool".to_string(),
        Type::Str => "str".to_string(),
        Type::Char => "char".to_string(),
        Type::Void => "void".to_string(),
        Type::Any => "any".to_string(),
        Type::Struct(name) => name.clone(),
        Type::Ptr(inner) => format!("*{}", type_to_string(inner)),
        Type::Array { element_type, size } => {
            format!("{}[{}]", type_to_string(element_type), size)
        }
        Type::GenericInstance { name, args } => {
            let args_str: Vec<String> = args.iter().map(|arg| type_to_string(arg)).collect();
            format!("{}<{}>", name, args_str.join(", "))
        }
    }
}
